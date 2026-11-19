//! CLI entry point: parse arguments, build the CA/store/proxy, and serve until interrupted.

use std::error::Error;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use rustls::crypto::ring::default_provider;
use tokio::net::TcpListener;

use proxyhand::api::Ui;
use proxyhand::ca::Ca;
use proxyhand::i18n::{self, Lang};
use proxyhand::proxy::Proxy;
use proxyhand::store::Store;
use proxyhand::tls::UpstreamTls;
use proxyhand::util;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_PORT: u16 = 7747;
const DEFAULT_MAX_FLOWS: usize = 3000;
const DEFAULT_MAX_BODY: usize = 8 * 1024 * 1024;

struct Options {
    port: u16,
    host: String,
    dir: PathBuf,
    no_ui: bool,
    verify_tls: bool,
    system_ca: bool,
    upstream_ca: Vec<String>,
    max_flows: usize,
    max_body: usize,
    self_hosts: Vec<String>,
    verbose: bool,
    lang: Lang,
}

fn main() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<(), Box<dyn Error>> {
    let options = match parse_args(std::env::args().skip(1).collect()) {
        Ok(Parse::Run(options)) => options,
        Ok(Parse::Help(lang)) => {
            print!("{}", i18n::help(lang));
            return Ok(());
        }
        Ok(Parse::Version) => {
            println!("proxyhand {VERSION}");
            return Ok(());
        }
        Err(message) => {
            eprintln!("proxyhand: {message}\n");
            eprint!("{}", i18n::help(Lang::from_env()));
            std::process::exit(2);
        }
    };

    let lang = options.lang;
    let provider = Arc::new(default_provider());

    // Read --upstream-ca files before anything binds, so a typo fails fast.
    let mut upstream_cas = Vec::new();
    for file in &options.upstream_ca {
        match std::fs::read_to_string(file) {
            Ok(pem) => upstream_cas.push(pem),
            Err(e) => {
                return Err(i18n::cannot_read_ca(lang, file, &e.to_string()).into());
            }
        }
    }

    let ca_cert_path = options.dir.join("ca.crt");
    let first_run = !ca_cert_path.exists();
    let ca = Ca::open(&options.dir, provider.clone())?;
    let tls = Arc::new(
        UpstreamTls::build(provider.clone(), options.system_ca, &upstream_cas)
            .map_err(|e| e.to_string())?,
    );

    // Bind first so the port shown in the banner is the port we actually got.
    let bind_ip: IpAddr = if options.host == "localhost" {
        "127.0.0.1".parse()?
    } else {
        options
            .host
            .parse()
            .unwrap_or(IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED))
    };
    let listener = TcpListener::bind(SocketAddr::new(bind_ip, options.port)).await?;
    let port = listener.local_addr()?.port();

    let store = Arc::new(Store::new(options.max_flows, VERSION));
    store.set_info(port, &ca_cert_path.to_string_lossy());

    let hosts = util::local_hosts(&options.host, &options.self_hosts);
    let ui = (!options.no_ui).then(|| {
        Arc::new(Ui {
            store: store.clone(),
            ca: ca.clone(),
            hosts: hosts.clone(),
            port,
            version: VERSION.to_string(),
            lang,
        })
    });

    let trusted_roots = tls.root_count;
    let proxy = Proxy::new(
        store.clone(),
        ca.clone(),
        tls,
        ui,
        port,
        options.verify_tls,
        options.max_body,
        options.verbose,
        lang,
    );

    print!(
        "{}",
        i18n::banner(
            lang,
            VERSION,
            &options.host,
            port,
            !options.no_ui,
            &ca_cert_path.to_string_lossy(),
            options.verify_tls,
            options.system_ca,
            upstream_cas.len(),
            trusted_roots,
        )
    );
    if first_run {
        println!("\n{}", i18n::first_run_hint(lang));
        for line in i18n::trust_instructions(lang, &ca_cert_path.to_string_lossy()) {
            println!("  {line}");
        }
    }
    println!();
    use std::io::Write;
    let _ = std::io::stdout().flush();

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let running = proxy.clone();
    let server = tokio::spawn(async move { running.run(listener, shutdown_rx).await });

    wait_for_signal().await;
    println!("\n{}", i18n::shutting_down(lang));
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), server).await;
    Ok(())
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

enum Parse {
    Run(Options),
    Help(Lang),
    Version,
}

fn parse_args(args: Vec<String>) -> Result<Parse, String> {
    let mut options = Options {
        port: DEFAULT_PORT,
        host: "127.0.0.1".to_string(),
        dir: default_dir(),
        no_ui: false,
        verify_tls: false,
        system_ca: true,
        upstream_ca: Vec::new(),
        max_flows: DEFAULT_MAX_FLOWS,
        max_body: DEFAULT_MAX_BODY,
        self_hosts: Vec::new(),
        verbose: false,
        lang: Lang::from_env(),
    };

    // Pick up --lang before parsing so `--lang zh --help` prints Chinese even though --help wins.
    if let Some(pos) = args.iter().position(|a| a == "--lang") {
        if let Some(value) = args.get(pos + 1).and_then(|v| Lang::parse(v)) {
            options.lang = value;
        }
    }
    for (pos, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--lang=").and_then(Lang::parse) {
            options.lang = value;
            let _ = pos;
        }
    }

    let mut iter = args.into_iter().peekable();
    while let Some(arg) = iter.next() {
        let mut value = |name: &str| -> Result<String, String> {
            iter.next().ok_or_else(|| format!("{name} needs a value"))
        };
        match arg.as_str() {
            "--help" => return Ok(Parse::Help(options.lang)),
            "-V" | "--version" => return Ok(Parse::Version),
            "-p" | "--port" => {
                let raw = value(&arg)?;
                options.port = raw.parse().map_err(|_| format!("invalid --port: {raw}"))?;
            }
            "-h" | "--host" => options.host = value(&arg)?,
            "-d" | "--dir" => options.dir = PathBuf::from(value(&arg)?),
            "--no-ui" => options.no_ui = true,
            "--verify-tls" => options.verify_tls = true,
            "--no-system-ca" => options.system_ca = false,
            "--max-flows" => {
                let raw = value(&arg)?;
                options.max_flows = raw
                    .parse()
                    .map_err(|_| format!("invalid --max-flows: {raw}"))?;
            }
            "--max-body" => {
                let raw = value(&arg)?;
                options.max_body = raw
                    .parse()
                    .map_err(|_| format!("invalid --max-body: {raw}"))?;
            }
            "--self-hosts" => {
                options.self_hosts = value(&arg)?
                    .split(',')
                    .map(|host| host.trim().to_string())
                    .filter(|host| !host.is_empty())
                    .collect();
            }
            "--upstream-ca" => options.upstream_ca.push(value(&arg)?),
            "--lang" => {
                let raw = value(&arg)?;
                options.lang = Lang::parse(raw.as_str())
                    .ok_or_else(|| format!("invalid --lang: {raw} (use en or zh)"))?;
            }
            "-v" | "--verbose" => options.verbose = true,
            other if other.starts_with("--lang=") => {
                let raw = &other["--lang=".len()..];
                options.lang = Lang::parse(raw)
                    .ok_or_else(|| format!("invalid --lang: {raw} (use en or zh)"))?;
            }
            other => return Err(format!("unknown option: {other}")),
        }
    }

    Ok(Parse::Run(options))
}

fn default_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("PROXYHAND_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".proxyhand")
}
