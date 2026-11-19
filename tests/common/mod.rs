//! Shared helpers for the integration tests: a proxy under test, small upstream
//! HTTP/HTTPS servers, and a raw HTTP/1.1 client that understands chunked bodies.
#![allow(dead_code)]

use std::convert::Infallible;
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, StreamBody, combinators::BoxBody};
use hyper::body::{Frame, Incoming};
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use proxyhand::api::Ui;
use proxyhand::ca::Ca;
use proxyhand::i18n::Lang;
use proxyhand::proxy::Proxy;
use proxyhand::store::Store;
use proxyhand::tls::UpstreamTls;
use rcgen::{
    BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use serde_json::Value;
use tempfile::TempDir;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tokio_rustls::TlsAcceptor;

pub type TestBody = BoxBody<Bytes, Infallible>;

const CLIENT_TIMEOUT: Duration = Duration::from_secs(10);

pub fn full_body(data: impl Into<Bytes>) -> TestBody {
    Full::new(data.into()).boxed()
}

pub struct ProxyHandle {
    pub addr: SocketAddr,
    pub store: Arc<Store>,
    pub ca_pem: String,
    _temp: TempDir,
    shutdown: watch::Sender<bool>,
    join: JoinHandle<()>,
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        self.join.abort();
    }
}

/// Start a real proxy on an ephemeral port.
///
/// `ui` controls whether requests addressed to the proxy port are served by the
/// built-in UI (and therefore never reach the loop guard).
pub async fn start_proxy(verify_tls: bool, extra_cas: &[String], ui_enabled: bool) -> ProxyHandle {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let temp = tempfile::tempdir().expect("tempdir");
    let ca = Ca::open(temp.path(), provider.clone()).expect("open test CA");
    let ca_pem = ca.cert_pem().to_string();
    let tls = Arc::new(
        UpstreamTls::build(provider, false, extra_cas).expect("build upstream TLS configs"),
    );
    let store = Arc::new(Store::new(500, "test"));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy listener");
    let addr = listener.local_addr().expect("proxy local addr");
    let port = addr.port();
    let ui = ui_enabled.then(|| {
        Arc::new(Ui {
            store: store.clone(),
            ca: ca.clone(),
            hosts: vec!["localhost".to_string(), "127.0.0.1".to_string()],
            port,
            version: "test".to_string(),
            lang: Lang::En,
        })
    });
    let proxy = Proxy::new(
        store.clone(),
        ca,
        tls,
        ui,
        port,
        verify_tls,
        4 * 1024 * 1024,
        false,
        Lang::En,
    );
    let (shutdown, rx) = watch::channel(false);
    let join = tokio::spawn(async move {
        proxy.run(listener, rx).await;
    });
    ProxyHandle {
        addr,
        store,
        ca_pem,
        _temp: temp,
        shutdown,
        join,
    }
}

/// A tiny upstream HTTP server. Routes: GET /hello, POST /echo, GET /sse,
/// GET /idle-sse; everything else is a 404.
pub struct HttpUpstream {
    pub addr: SocketAddr,
    join: JoinHandle<()>,
}

impl Drop for HttpUpstream {
    fn drop(&mut self) {
        self.join.abort();
    }
}

async fn http_service(req: Request<Incoming>) -> Result<Response<TestBody>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let response = match (method, path.as_str()) {
        (Method::GET, "/hello") => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/plain; charset=utf-8")
            .header("x-upstream", "proxyhand-test")
            .body(full_body("hello from upstream")),
        (Method::POST, "/echo") => {
            let body = req
                .into_body()
                .collect()
                .await
                .expect("read upstream request body")
                .to_bytes();
            let payload = serde_json::json!({
                "method": "POST",
                "body": String::from_utf8_lossy(&body),
            });
            Response::builder()
                .status(StatusCode::OK)
                .header("content-type", "application/json")
                .body(full_body(payload.to_string()))
        }
        (Method::GET, "/sse") => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .body(sse_body()),
        (Method::GET, "/idle-sse") => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(idle_sse_body()),
        _ => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .header("content-type", "text/plain")
            .body(full_body("not found")),
    };
    Ok(response.expect("build upstream response"))
}

fn sse_body() -> TestBody {
    let frames: Vec<&'static [u8]> = vec![
        b"event: task\ndata: {\"n\":1}\n\n",
        b"id: 7\ndata: {\"n\":2}\n\n",
        b"data: [done]\n\n",
    ];
    let stream = futures_util::stream::unfold(frames.into_iter(), |mut frames| async move {
        let frame = frames.next()?;
        tokio::time::sleep(Duration::from_millis(15)).await;
        Some((
            Ok::<_, Infallible>(Frame::data(Bytes::from_static(frame))),
            frames,
        ))
    });
    StreamBody::new(stream).boxed()
}

fn idle_sse_body() -> TestBody {
    let first = futures_util::stream::once(async {
        Ok::<_, Infallible>(Frame::data(Bytes::from_static(b": open\n\n")))
    });
    let rest = futures_util::stream::pending::<Result<Frame<Bytes>, Infallible>>();
    StreamBody::new(futures_util::StreamExt::chain(first, rest)).boxed()
}

pub async fn start_http_upstream() -> HttpUpstream {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind upstream listener");
    let addr = listener.local_addr().expect("upstream addr");
    let join = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let io = TokioIo::new(tcp);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service_fn(http_service))
                    .await;
            });
        }
    });
    HttpUpstream { addr, join }
}

/// An HTTPS upstream whose certificate chain is signed by a throwaway test CA.
pub struct HttpsUpstream {
    pub addr: SocketAddr,
    pub ca_pem: String,
    join: JoinHandle<()>,
}

impl Drop for HttpsUpstream {
    fn drop(&mut self) {
        self.join.abort();
    }
}

async fn secure_service(req: Request<Incoming>) -> Result<Response<TestBody>, Infallible> {
    let body = format!("secure:{}", req.uri().path());
    Ok(Response::builder()
        .status(StatusCode::OK)
        .header("content-type", "text/plain; charset=utf-8")
        .body(full_body(body))
        .expect("build secure response"))
}

fn tls_material() -> (Arc<ServerConfig>, String) {
    let ca_key = KeyPair::generate().expect("generate CA key");
    let mut ca_params = CertificateParams::new(Vec::new()).expect("CA params");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params.key_usages = vec![
        KeyUsagePurpose::KeyCertSign,
        KeyUsagePurpose::CrlSign,
        KeyUsagePurpose::DigitalSignature,
    ];
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "proxyhand test upstream CA");
    ca_params.use_authority_key_identifier_extension = true;
    let ca_cert = ca_params.self_signed(&ca_key).expect("self-sign test CA");

    let leaf_key = KeyPair::generate().expect("generate leaf key");
    let mut leaf_params =
        CertificateParams::new(vec!["localhost".to_string()]).expect("leaf params");
    leaf_params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    leaf_params.use_authority_key_identifier_extension = true;
    let issuer = Issuer::from_params(&ca_params, &ca_key);
    let leaf_cert = leaf_params
        .signed_by(&leaf_key, &issuer)
        .expect("sign leaf certificate");

    let chain = vec![CertificateDer::from(leaf_cert.der().to_vec())];
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der()));
    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("safe TLS versions")
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .expect("server config");
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    (Arc::new(config), ca_cert.pem())
}

pub async fn start_https_upstream() -> HttpsUpstream {
    let (config, ca_pem) = tls_material();
    let acceptor = TlsAcceptor::from(config);
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind HTTPS upstream listener");
    let addr = listener.local_addr().expect("HTTPS upstream addr");
    let join = tokio::spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                break;
            };
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let Ok(tls) = acceptor.accept(tcp).await else {
                    return;
                };
                let io = TokioIo::new(tls);
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, service_fn(secure_service))
                    .await;
            });
        }
    });
    HttpsUpstream { addr, ca_pem, join }
}

pub struct HttpResponse {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|err| panic!("response is not JSON ({err}): {}", self.text()))
    }
}

pub struct ResponseHead {
    pub status: u16,
    pub reason: String,
    pub headers: Vec<(String, String)>,
    pub leftover: Vec<u8>,
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

async fn fill<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut Vec<u8>) -> io::Result<usize> {
    let mut chunk = [0u8; 4096];
    let read = stream.read(&mut chunk).await?;
    buf.extend_from_slice(&chunk[..read]);
    Ok(read)
}

/// Read and parse a response head, leaving any body bytes already read in
/// `leftover`. Used by the idle-SSE regression test, which cannot use the
/// regular full-response reader because the body never ends.
pub async fn read_response_head<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<ResponseHead> {
    let mut buf = Vec::new();
    let head_end = loop {
        if let Some(pos) = find_subslice(&buf, b"\r\n\r\n") {
            break pos;
        }
        if fill(stream, &mut buf).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed before response headers",
            ));
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let leftover = buf[head_end + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status_line = lines.next().unwrap_or_default();
    let mut parts = status_line.splitn(3, ' ');
    let _version = parts.next().unwrap_or_default();
    let status = parts.next().unwrap_or("0").parse().unwrap_or(0);
    let reason = parts.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect();
    Ok(ResponseHead {
        status,
        reason,
        headers,
        leftover,
    })
}

async fn take_line<S: AsyncRead + Unpin>(stream: &mut S, buf: &mut Vec<u8>) -> io::Result<String> {
    loop {
        if let Some(pos) = find_subslice(buf, b"\r\n") {
            let line = String::from_utf8_lossy(&buf[..pos]).into_owned();
            buf.drain(..pos + 2);
            return Ok(line);
        }
        if fill(stream, buf).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "connection closed in chunked body",
            ));
        }
    }
}

async fn read_chunked<S: AsyncRead + Unpin>(
    stream: &mut S,
    mut buf: Vec<u8>,
) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line = take_line(stream, &mut buf).await?;
        let size_text = line.split(';').next().unwrap_or_default().trim();
        let size = usize::from_str_radix(size_text, 16).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid chunk size {size_text:?}"),
            )
        })?;
        if size == 0 {
            while !take_line(stream, &mut buf).await?.is_empty() {}
            return Ok(body);
        }
        while buf.len() < size + 2 {
            if fill(stream, &mut buf).await? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "connection closed in chunk data",
                ));
            }
        }
        body.extend_from_slice(&buf[..size]);
        buf.drain(..size + 2);
    }
}

async fn read_full_response<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<HttpResponse> {
    let ResponseHead {
        status,
        reason,
        headers,
        leftover,
    } = read_response_head(stream).await?;
    let chunked = headers.iter().any(|(name, value)| {
        name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked")
    });
    let content_length = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.trim().parse::<usize>().ok());
    let body = if chunked {
        read_chunked(stream, leftover).await?
    } else if let Some(length) = content_length {
        let mut body = leftover;
        while body.len() < length {
            if fill(stream, &mut body).await? == 0 {
                break;
            }
        }
        body.truncate(length);
        body
    } else {
        let mut body = leftover;
        while fill(stream, &mut body).await? > 0 {}
        body
    };
    Ok(HttpResponse {
        status,
        reason,
        headers,
        body,
    })
}

pub async fn connect(addr: SocketAddr) -> TcpStream {
    timeout(CLIENT_TIMEOUT, TcpStream::connect(addr))
        .await
        .expect("connecting to the proxy timed out")
        .expect("connect to proxy")
}

/// Send a raw HTTP/1.1 request and read the complete response.
///
/// `target` is used verbatim, so it can be an origin-form path (`/api/flows`)
/// or an absolute-form URL (`http://host:port/path`).
pub async fn request(
    addr: SocketAddr,
    method: &str,
    target: &str,
    host: &str,
    extra_headers: &[(&str, &str)],
    body: &[u8],
) -> HttpResponse {
    let mut stream = connect(addr).await;
    let mut raw = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    for (name, value) in extra_headers {
        raw.push_str(name);
        raw.push_str(": ");
        raw.push_str(value);
        raw.push_str("\r\n");
    }
    if !body.is_empty() || method == "POST" {
        raw.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    raw.push_str("\r\n");
    let mut bytes = raw.into_bytes();
    bytes.extend_from_slice(body);
    stream.write_all(&bytes).await.expect("write request");
    timeout(CLIENT_TIMEOUT, read_full_response(&mut stream))
        .await
        .expect("reading proxy response timed out")
        .expect("read proxy response")
}

pub async fn api_get(proxy: SocketAddr, path: &str) -> HttpResponse {
    request(
        proxy,
        "GET",
        path,
        &format!("127.0.0.1:{}", proxy.port()),
        &[],
        &[],
    )
    .await
}

pub async fn api_get_json(proxy: SocketAddr, path: &str) -> (u16, Value) {
    let response = api_get(proxy, path).await;
    let value = response.json();
    (response.status, value)
}

/// POST a JSON body to one of the UI endpoints.
pub async fn api_post_json(proxy: SocketAddr, path: &str, body: &Value) -> (u16, Value) {
    let host = proxy.to_string();
    let payload = serde_json::to_vec(body).expect("encode json body");
    let response = request(
        proxy,
        "POST",
        path,
        &host,
        &[("content-type", "application/json")],
        &payload,
    )
    .await;
    (response.status, response.json())
}

/// Poll `/api/flows/{id}` until `accept` returns true.
pub async fn wait_for_flow<F>(proxy: SocketAddr, id: u64, mut accept: F) -> Value
where
    F: FnMut(&Value) -> bool,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let path = format!("/api/flows/{id}");
    loop {
        let (status, value) = api_get_json(proxy, &path).await;
        if status == 200 && accept(&value) {
            return value;
        }
        if tokio::time::Instant::now() > deadline {
            panic!("timed out waiting for flow {id}; last response: {value}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub async fn proxy_get(proxy: SocketAddr, url: &str) -> HttpResponse {
    let authority = url
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or_default();
    request(proxy, "GET", url, authority, &[], &[]).await
}

/// Perform an HTTPS request through the proxy's CONNECT tunnel, trusting the
/// proxy's dynamically generated CA exactly like a configured browser would.
pub async fn https_get_via_proxy(
    proxy: SocketAddr,
    proxy_ca_pem: &str,
    authority: &str,
    path: &str,
) -> HttpResponse {
    let mut stream = connect(proxy).await;
    let connect_request = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nConnection: keep-alive\r\n\r\n"
    );
    stream
        .write_all(connect_request.as_bytes())
        .await
        .expect("write CONNECT");
    let head = timeout(CLIENT_TIMEOUT, read_response_head(&mut stream))
        .await
        .expect("CONNECT response timed out")
        .expect("read CONNECT response");
    assert_eq!(head.status, 200, "CONNECT was rejected: {}", head.reason);

    let der = proxyhand::ca::pem_to_der(proxy_ca_pem).expect("proxy CA PEM");
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(der))
        .expect("add proxy CA to client roots");
    let config =
        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("client TLS versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let server_name =
        rustls::pki_types::ServerName::try_from("localhost".to_string()).expect("server name");
    let mut tls = timeout(CLIENT_TIMEOUT, connector.connect(server_name, stream))
        .await
        .expect("TLS handshake with proxy timed out")
        .expect("TLS handshake with proxy");

    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    tls.write_all(request.as_bytes())
        .await
        .expect("write HTTPS request");
    timeout(CLIENT_TIMEOUT, read_full_response(&mut tls))
        .await
        .expect("reading HTTPS response timed out")
        .expect("read HTTPS response")
}
