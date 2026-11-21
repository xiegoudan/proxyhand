//! The proxy: the listener, CONNECT tunnels, upstream requests and body capture.
//!
//! Flow through one proxied exchange:
//!
//! ```text
//! client ──► listener ──► handle() ──► handle_http() ──► dial upstream ──► hyper client
//!                                └──► handle_connect() ──► TLS terminate ──► serve_connection()
//! response head ◄── flow row + flow:update ◄── CaptureBody tee ◄── hyper client response
//! ```

use std::collections::HashMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use futures_util::{Stream, StreamExt};
use http::header::{self, HeaderMap, HeaderName};
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt, BodyStream, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::client::conn::http1 as http1_client;
use hyper::server::conn::http1 as http1_server;
use hyper::service::service_fn;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_rustls::client::TlsStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};

use crate::api::{self, Ui};
use crate::ca::Ca;
use crate::i18n::{self, Lang};
use crate::sse::{NdjsonParser, SseParser};
use crate::store::{Flow, FlowSummary, Kind, Note, Patch, Store};
use crate::tls::UpstreamTls;
use crate::util::{self, BodyView, Target};
use crate::ws::{Direction, WsParser};

/// Request bodies above this are refused instead of being buffered in memory.
const MAX_REQUEST_BODY: usize = 64 * 1024 * 1024;
/// How long an upstream may take to connect, handshake and send response headers.
const HEADER_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a non-streaming response body may sit idle before the transfer is abandoned.
const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Name resolution results are reused this long: a browser opens many connections per page, and
/// one getaddrinfo round trip per connection is the difference between instant and sluggish.
const DNS_TTL: Duration = Duration::from_secs(60);
/// A failed lookup is remembered briefly, so a broken name does not stall every retry.
const DNS_FAILURE_TTL: Duration = Duration::from_secs(5);
/// One address may take this long to accept a connection before the next one is tried.
const ADDR_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// A CONNECT tunnel handed to the tunnel worker. The hyper `Upgraded` future cannot be awaited
/// inside the CONNECT service future itself: it would make `handle()`\x27s `Send` proof depend on
/// `serve_connection()`, which in turn needs `handle()` — a cycle rustc refuses to break.
type TunnelJob = (hyper::upgrade::OnUpgrade, String, String);

pub struct Proxy {
    pub store: Arc<Store>,
    pub ca: Arc<Ca>,
    pub tls: Arc<UpstreamTls>,
    pub ui: Option<Arc<Ui>>,
    pub port: u16,
    pub verify_tls: bool,
    pub max_body: usize,
    pub verbose: bool,
    pub lang: Lang,
    tunnels: mpsc::UnboundedSender<TunnelJob>,
    tunnel_rx: std::sync::Mutex<Option<mpsc::UnboundedReceiver<TunnelJob>>>,
    /// host → addresses, with the moment they were resolved.
    dns: Mutex<HashMap<String, (std::time::Instant, Vec<SocketAddr>)>>,
}

impl Proxy {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        store: Arc<Store>,
        ca: Arc<Ca>,
        tls: Arc<UpstreamTls>,
        ui: Option<Arc<Ui>>,
        port: u16,
        verify_tls: bool,
        max_body: usize,
        verbose: bool,
        lang: Lang,
    ) -> Arc<Proxy> {
        let (tunnels, tunnel_rx) = mpsc::unbounded_channel();
        Arc::new(Proxy {
            store,
            ca,
            tls,
            ui,
            port,
            verify_tls,
            max_body,
            verbose,
            lang,
            tunnels,
            tunnel_rx: std::sync::Mutex::new(Some(tunnel_rx)),
            dns: Mutex::new(HashMap::new()),
        })
    }

    fn log(&self, message: &str) {
        if self.verbose {
            eprintln!("[proxyhand] {message}");
        }
    }

    /// Accepts connections until `shutdown` flips.
    pub async fn run(
        self: Arc<Self>,
        listener: TcpListener,
        mut shutdown: tokio::sync::watch::Receiver<bool>,
    ) {
        if let Some(mut jobs) = self.tunnel_rx.lock().ok().and_then(|mut slot| slot.take()) {
            let worker = self.clone();
            tokio::spawn(async move {
                while let Some((upgrade, authority, client_ip)) = jobs.recv().await {
                    let conn = worker.clone();
                    tokio::spawn(async move {
                        match upgrade.await {
                            Ok(upgraded) => conn.serve_tunnel(upgraded, authority, client_ip).await,
                            Err(e) => conn.log(&format!("CONNECT upgrade failed: {e}")),
                        }
                    });
                }
            });
        }
        loop {
            tokio::select! {
                _ = shutdown.changed() => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, peer)) => {
                        let this = self.clone();
                        tokio::spawn(async move {
                            this.serve_connection(stream, false, None, peer.ip().to_string()).await;
                        });
                    }
                    Err(e) => {
                        self.log(&format!("accept failed: {e}"));
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                },
            }
        }
    }

    /// Serves HTTP/1.1 requests read from `io` (plain or TLS, depending on `is_tls`).
    async fn serve_connection<I>(
        self: Arc<Self>,
        io: I,
        is_tls: bool,
        hint: Option<String>,
        client_ip: String,
    ) where
        I: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let service = {
            let this = self.clone();
            let hint = hint.clone();
            let client_ip = client_ip.clone();
            service_fn(move |req: Request<Incoming>| {
                let this = this.clone();
                let hint = hint.clone();
                let client_ip = client_ip.clone();
                async move {
                    Ok::<Response<api::Body>, std::convert::Infallible>(
                        this.handle(req, is_tls, hint, client_ip).await,
                    )
                }
            })
        };
        let conn = http1_server::Builder::new()
            .serve_connection(TokioIo::new(io), service)
            .with_upgrades();
        if let Err(e) = conn.await {
            self.log(&format!("connection closed: {e}"));
        }
    }

    async fn handle(
        self: Arc<Self>,
        req: Request<Incoming>,
        is_tls: bool,
        hint: Option<String>,
        client_ip: String,
    ) -> Response<api::Body> {
        if self.verbose {
            eprintln!(
                "proxyhand: {} {} host={:?}",
                req.method(),
                req.uri(),
                req.headers().get("host")
            );
        }
        if req.method() == Method::CONNECT {
            return self.handle_connect(req, client_ip).await;
        }
        if let Some(ui) = self.ui.clone() {
            // Two ways to reach the UI: through the proxy on its own port
            // (absolute-form, port must match), or straight at it from a browser
            // (origin-form). The latter ignores the port, so publishing a
            // container on `-p 8899:7747` keeps http://127.0.0.1:8899 working.
            let origin_form = req.uri().scheme().is_none() && req.uri().authority().is_none();
            let hit = match destination(&req, self.port) {
                Some((host, port)) => {
                    ui.serves(&host, port) || (origin_form && !is_tls && ui.serves_host(&host))
                }
                None => false,
            };
            if hit {
                if self.verbose {
                    let (host, port) = destination(&req, self.port).unwrap_or_default();
                    eprintln!("proxyhand: ui {host}:{port} {}", req.uri().path());
                }
                return ui.handle(req).await;
            }
        }
        self.handle_http(req, is_tls, hint, client_ip).await
    }

    async fn handle_connect(
        self: Arc<Self>,
        mut req: Request<Incoming>,
        client_ip: String,
    ) -> Response<api::Body> {
        let authority = req
            .uri()
            .authority()
            .map(|a| a.to_string())
            .unwrap_or_default();
        if authority.is_empty() {
            return text_response(StatusCode::BAD_REQUEST, "CONNECT without authority");
        }
        let upgrade = hyper::upgrade::on(&mut req);
        if self.tunnels.send((upgrade, authority, client_ip)).is_err() {
            return text_response(StatusCode::SERVICE_UNAVAILABLE, "proxy is shutting down");
        }
        Response::builder()
            .status(StatusCode::OK)
            .body(api::body_from(Bytes::new()))
            .unwrap()
    }

    /// Decides what lives inside a CONNECT tunnel: TLS or raw bytes.
    async fn serve_tunnel(
        self: Arc<Self>,
        upgraded: Upgraded,
        authority: String,
        client_ip: String,
    ) {
        let (host, port) = util::split_host_port(authority.trim(), 443);
        let io = TokioIo::new(upgraded);
        let mut first = [0u8; 1];
        let peek = {
            let mut probe = PrefixedIo::new(BytesMut::new(), io);
            match probe.read_exact(&mut first).await {
                Ok(_) => {
                    let io = probe.into_inner();
                    Some((first[0], io))
                }
                Err(_) => None,
            }
        };
        let Some((byte, io)) = peek else { return };

        if byte == 0x16 {
            // TLS ClientHello: terminate it so the traffic inside is visible.
            let stream = PrefixedIo::new(BytesMut::from(&first[..]), io);
            let acceptor = TlsAcceptor::from(self.ca.server_config());
            match acceptor.accept(stream).await {
                Ok(tls) => {
                    self.serve_connection(tls, true, Some(authority.clone()), client_ip)
                        .await;
                }
                Err(e) => self.log(&format!("tunnel TLS handshake failed for {authority}: {e}")),
            }
            return;
        }

        // Anything else inside CONNECT is forwarded byte for byte.
        let flow_id = self
            .store
            .create(tunnel_flow(&authority, &host, port, &client_ip))
            .map(|f| f.summary.id);
        // Same resolver as the HTTP path: one lookup per tunnel would otherwise repeat the IPv6
        // and EAI_AGAIN problems for every WebSocket-over-TLS client that opens a tunnel.
        let addrs = match self.resolve(&host, port).await {
            Ok(addrs) => addrs,
            Err(e) => {
                self.fail(
                    flow_id,
                    StatusCode::BAD_GATEWAY,
                    i18n::upstream_error(self.lang, &e),
                );
                return;
            }
        };
        let mut upstream = match connect_any(&addrs).await {
            Ok(stream) => stream,
            Err(e) => {
                self.fail(
                    flow_id,
                    StatusCode::BAD_GATEWAY,
                    i18n::upstream_error(self.lang, &e),
                );
                return;
            }
        };
        if upstream.write_all(&first).await.is_err() {
            return;
        }
        let mut io = io;
        let (mut client_read, mut client_write) = tokio::io::split(&mut io);
        let (mut up_read, mut up_write) = tokio::io::split(&mut upstream);
        let (incoming, outgoing) = tokio::join!(
            pipe(&mut client_read, &mut up_write),
            pipe(&mut up_read, &mut client_write),
        );
        if let Some(id) = flow_id {
            self.store.update(
                id,
                Patch {
                    open: Some(false),
                    ended_at: Some(util::now_ms()),
                    req_bytes: Some(incoming),
                    res_bytes: Some(outgoing),
                    size: Some(outgoing),
                    ..Default::default()
                },
            );
            self.store.add_bytes_in(incoming);
            self.store.add_bytes_out(outgoing);
        }
    }

    /// One proxied HTTP request, with capture.
    async fn handle_http(
        self: Arc<Self>,
        mut req: Request<Incoming>,
        is_tls: bool,
        hint: Option<String>,
        client_ip: String,
    ) -> Response<api::Body> {
        let lang = self.lang;
        let started_at = util::now_ms();
        let upgrading = util::is_upgrade(req.headers());
        let loop_marked = req.headers().contains_key("x-proxyhand-loop");
        let hint_authority: Option<http::uri::Authority> =
            hint.as_deref().and_then(|h| h.parse().ok());

        let Some(target) =
            util::resolve_target(req.uri(), req.headers(), is_tls, hint_authority.as_ref())
        else {
            return text_response(
                StatusCode::BAD_REQUEST,
                &i18n::no_route(lang, req.method().as_str(), req.uri().path()),
            );
        };

        if target.port == self.port && loop_marked {
            return text_response(StatusCode::LOOP_DETECTED, &i18n::loop_detected(lang));
        }

        let downstream_upgrade = upgrading.then(|| hyper::upgrade::on(&mut req));
        let (parts, body) = req.into_parts();

        // Buffered request body: makes the "verification failed, retry insecure" path simple,
        // because a request that fails at the TLS handshake was never consumed.
        let mut req_raw: Vec<u8> = Vec::new();
        if !upgrading {
            if let Some(len) = parts
                .headers
                .get(header::CONTENT_LENGTH)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<usize>().ok())
            {
                if len > MAX_REQUEST_BODY {
                    return text_response(
                        StatusCode::PAYLOAD_TOO_LARGE,
                        &i18n::body_too_large(lang, MAX_REQUEST_BODY),
                    );
                }
            }
            match body.collect().await {
                Ok(collected) => req_raw = collected.to_bytes().to_vec(),
                Err(e) => {
                    return text_response(
                        StatusCode::BAD_REQUEST,
                        &format!("request body could not be read: {e}"),
                    );
                }
            }
        }

        // A bodyless request (the usual GET) must not grow an empty body view: the UI would show an
        // empty "request body" section for every page load.
        let req_view = if req_raw.is_empty() {
            None
        } else {
            let req_content_type = header_str(&parts.headers, header::CONTENT_TYPE);
            let req_encoding = header_str(&parts.headers, header::CONTENT_ENCODING);
            util::build_body(
                &[req_raw.clone()],
                req_raw.len() as u64,
                0,
                req_content_type.as_deref(),
                req_encoding.as_deref(),
                self.max_body,
            )
        };

        // Upstream request.
        let mut headers = HeaderMap::new();
        for (name, value) in parts.headers.iter() {
            let n = name.as_str();
            if n == "host" || n == "content-length" {
                continue;
            }
            if util::hop_by_hop(n) && !(upgrading && n == "upgrade") {
                continue;
            }
            headers.append(name.clone(), value.clone());
        }
        headers.insert(header::HOST, util::header_value(&target.authority));
        headers.insert(
            header::CONNECTION,
            util::header_value(if upgrading { "Upgrade" } else { "keep-alive" }),
        );
        if target.port == self.port {
            headers.insert(
                HeaderName::from_static("x-proxyhand-loop"),
                util::header_value("1"),
            );
        }

        let mut builder = Request::builder()
            .method(parts.method.clone())
            .uri(if target.path.is_empty() {
                "/"
            } else {
                target.path.as_str()
            })
            .version(http::Version::HTTP_11);
        for (name, value) in headers.iter() {
            builder = builder.header(name, value);
        }
        let upstream_req = match builder.body(Full::new(Bytes::from(req_raw.clone()))) {
            Ok(req) => req,
            Err(e) => {
                return text_response(StatusCode::BAD_REQUEST, &format!("invalid request: {e}"));
            }
        };

        let kind = if upgrading { Kind::Ws } else { Kind::Http };
        let flow_id = self
            .store
            .create(new_flow(
                &target,
                &parts.method,
                kind,
                started_at,
                &client_ip,
                req_view,
                util::header_pairs(&parts.headers),
            ))
            .map(|flow| flow.summary.id);

        // Dial. One timeout covers connect + TLS + response headers.
        let dialed = match timeout(HEADER_TIMEOUT, self.dial(&target)).await {
            Ok(Ok(dialed)) => dialed,
            Ok(Err(message)) => {
                return self.fail(
                    flow_id,
                    StatusCode::BAD_GATEWAY,
                    i18n::upstream_error(lang, &message),
                );
            }
            Err(_) => {
                return self.fail(
                    flow_id,
                    StatusCode::GATEWAY_TIMEOUT,
                    i18n::header_timeout(lang, HEADER_TIMEOUT.as_secs()),
                );
            }
        };
        if let (Some(id), Some(reason)) = (flow_id, dialed.verify_fallback.as_ref()) {
            self.store.add_note(id, Note::TlsFallback(reason.clone()));
        }
        if let (Some(id), Some(alpn)) = (flow_id, dialed.alpn.as_ref()) {
            self.store.update(
                id,
                Patch {
                    alpn: Some(alpn.clone()),
                    ..Default::default()
                },
            );
        }
        if let Some(id) = flow_id {
            self.store.update(
                id,
                Patch {
                    open: Some(true),
                    ..Default::default()
                },
            );
        }

        let io = TokioIo::new(dialed.io);
        let (mut sender, conn) = match http1_client::handshake(io).await {
            Ok(pair) => pair,
            Err(e) => {
                return self.fail(
                    flow_id,
                    StatusCode::BAD_GATEWAY,
                    i18n::upstream_error(lang, &e.to_string()),
                );
            }
        };
        let verbose = self.verbose;
        tokio::spawn(async move {
            if let Err(e) = conn.with_upgrades().await {
                if verbose {
                    eprintln!("[proxyhand] upstream connection closed: {e}");
                }
            }
        });

        let mut resp = match timeout(HEADER_TIMEOUT, sender.send_request(upstream_req)).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => {
                return self.fail(
                    flow_id,
                    StatusCode::BAD_GATEWAY,
                    i18n::upstream_error(lang, &e.to_string()),
                );
            }
            Err(_) => {
                return self.fail(
                    flow_id,
                    StatusCode::GATEWAY_TIMEOUT,
                    i18n::header_timeout(lang, HEADER_TIMEOUT.as_secs()),
                );
            }
        };
        let ttfb = util::now_ms().saturating_sub(started_at);
        let status = resp.status();
        let res_content_type = header_str(resp.headers(), header::CONTENT_TYPE);
        let res_encoding = header_str(resp.headers(), header::CONTENT_ENCODING);
        let res_headers = util::header_pairs(resp.headers());

        let upgrading = upgrading && status == StatusCode::SWITCHING_PROTOCOLS;
        let upstream_upgrade = upgrading.then(|| hyper::upgrade::on(&mut resp));

        let (res_parts, res_body) = resp.into_parts();
        let content_type = header_str(&res_parts.headers, header::CONTENT_TYPE);
        let encoding = header_str(&res_parts.headers, header::CONTENT_ENCODING);
        let kind = if upgrading {
            Kind::Ws
        } else if util::is_streaming(content_type.as_deref()) {
            Kind::Sse
        } else if util::is_ndjson(content_type.as_deref()) {
            Kind::Ndjson
        } else {
            Kind::Http
        };

        if let Some(id) = flow_id {
            self.store.update(
                id,
                Patch {
                    status: Some(status.as_u16()),
                    status_text: Some(status.canonical_reason().unwrap_or("").to_string()),
                    kind: Some(kind),
                    content_type: content_type.clone(),
                    res_headers: Some(res_headers),
                    ttfb_ms: Some(ttfb),
                    ..Default::default()
                },
            );
        }

        let mut out_headers = HeaderMap::new();
        for (name, value) in res_parts.headers.iter() {
            let n = name.as_str();
            if util::hop_by_hop(n) && !(upgrading && matches!(n, "upgrade" | "connection")) {
                continue;
            }
            if n == "content-length" && upgrading {
                continue;
            }
            out_headers.append(name.clone(), value.clone());
        }
        let _ = (res_content_type, res_encoding);

        if upgrading {
            if let (Some(down), Some(up)) = (downstream_upgrade, upstream_upgrade) {
                let this = self.clone();
                let ws_id = flow_id;
                tokio::spawn(async move {
                    match tokio::join!(down, up) {
                        (Ok(down), Ok(up)) => this.pipe_websocket(ws_id, down, up).await,
                        _ => {
                            if let Some(id) = ws_id {
                                this.store.update(
                                    id,
                                    Patch {
                                        open: Some(false),
                                        ended_at: Some(util::now_ms()),
                                        ..Default::default()
                                    },
                                );
                            }
                        }
                    }
                });
            }
            let mut builder = Response::builder().status(status);
            for (name, value) in out_headers.iter() {
                builder = builder.header(name, value);
            }
            return builder.body(api::body_from(Bytes::new())).unwrap();
        }

        let streaming = matches!(kind, Kind::Sse | Kind::Ndjson);
        let body = self.spawn_body_pump(
            flow_id,
            res_body,
            kind,
            content_type,
            encoding,
            streaming,
            started_at,
        );

        let mut builder = Response::builder().status(status);
        for (name, value) in out_headers.iter() {
            builder = builder.header(name, value);
        }
        let no_body = parts.method == Method::HEAD
            || status == StatusCode::NO_CONTENT
            || status == StatusCode::NOT_MODIFIED;
        let body = if no_body {
            api::body_from(Bytes::new())
        } else {
            body
        };
        builder.body(body).unwrap()
    }

    fn fail(
        &self,
        flow_id: Option<u64>,
        status: StatusCode,
        message: String,
    ) -> Response<api::Body> {
        if let Some(id) = flow_id {
            self.store.update(
                id,
                Patch {
                    status: Some(status.as_u16()),
                    status_text: Some(status.canonical_reason().unwrap_or("").to_string()),
                    error: Some(message.clone()),
                    open: Some(false),
                    ended_at: Some(util::now_ms()),
                    ..Default::default()
                },
            );
        }
        text_response(status, &message)
    }

    /// Streams the upstream body to the client while tapping it for the UI.
    #[allow(clippy::too_many_arguments)]
    fn spawn_body_pump(
        &self,
        flow_id: Option<u64>,
        incoming: Incoming,
        kind: Kind,
        content_type: Option<String>,
        encoding: Option<String>,
        streaming: bool,
        _started_at: u64,
    ) -> api::Body {
        let (tx, rx) = mpsc::channel::<Result<Frame<Bytes>, io::Error>>(8);
        let store = self.store.clone();
        let lang = self.lang;
        let mut tap = BodyTap::new(kind, content_type, encoding, self.max_body);
        tokio::spawn(async move {
            let mut stream = BodyStream::new(incoming);
            let mut failure: Option<String> = None;
            loop {
                let next = if streaming {
                    stream.next().await
                } else {
                    match timeout(BODY_IDLE_TIMEOUT, stream.next()).await {
                        Ok(item) => item,
                        Err(_) => {
                            failure = Some(i18n::body_stalled(lang, BODY_IDLE_TIMEOUT.as_secs()));
                            break;
                        }
                    }
                };
                let Some(item) = next else { break };
                let frame = match item {
                    Ok(frame) => frame,
                    Err(e) => {
                        failure = Some(e.to_string());
                        break;
                    }
                };
                if let Some(data) = frame.data_ref() {
                    tap.data(data, &store, flow_id);
                }
                if tx.send(Ok(frame)).await.is_err() {
                    // The client went away; keep what was captured so far.
                    break;
                }
            }
            let (view, total, dropped) = tap.finish();
            if let Some(id) = flow_id {
                store.add_bytes_out(total);
                if dropped > 0 {
                    store.add_note(id, Note::BodyTruncated);
                }
                store.update(
                    id,
                    Patch {
                        open: Some(false),
                        ended_at: Some(util::now_ms()),
                        res_body: view,
                        size: Some(total),
                        res_bytes: Some(total),
                        error: failure,
                        ..Default::default()
                    },
                );
            }
        });
        BodyExt::boxed(StreamBody::new(ChannelStream { rx }))
    }

    async fn pipe_websocket(self: Arc<Self>, flow_id: Option<u64>, down: Upgraded, up: Upgraded) {
        let mut down = TokioIo::new(down);
        let mut up = TokioIo::new(up);
        let (mut down_read, mut down_write) = tokio::io::split(&mut down);
        let (mut up_read, mut up_write) = tokio::io::split(&mut up);

        let store = self.store.clone();
        let c2s_store = store.clone();
        let c2s = pipe_tapped(&mut down_read, &mut up_write, move |data| {
            let mut parser = WsParser::new(Direction::ClientToServer);
            parser.feed(data, &mut |frame| {
                if let Some(id) = flow_id {
                    c2s_store.add_ws(id, frame);
                }
            });
        });
        let s2c_store = store.clone();
        let s2c = pipe_tapped(&mut up_read, &mut down_write, move |data| {
            let mut parser = WsParser::new(Direction::ServerToClient);
            parser.feed(data, &mut |frame| {
                if let Some(id) = flow_id {
                    s2c_store.add_ws(id, frame);
                }
            });
        });
        let (incoming, outgoing) = tokio::join!(c2s, s2c);
        if let Some(id) = flow_id {
            store.add_bytes_in(incoming);
            store.add_bytes_out(outgoing);
            store.update(
                id,
                Patch {
                    open: Some(false),
                    ended_at: Some(util::now_ms()),
                    req_bytes: Some(incoming),
                    res_bytes: Some(outgoing),
                    size: Some(outgoing),
                    ..Default::default()
                },
            );
        }
    }

    async fn dial(&self, target: &Target) -> Result<Dialed, String> {
        let mut addrs = self.resolve(&target.host, target.port).await?;
        let tcp = match connect_any(&addrs).await {
            Ok(tcp) => tcp,
            Err(first) => {
                // A cached answer can go stale — fake-IP resolvers rotate their mappings, and a
                // load-balanced pool drops addresses. Ask again once before giving up.
                self.forget(&target.host);
                match self.refresh(&target.host, target.port).await {
                    Ok(fresh) => {
                        addrs = fresh;
                        connect_any(&addrs)
                            .await
                            .map_err(|second| format!("{first}; after a fresh lookup: {second}"))?
                    }
                    Err(e) => return Err(format!("{first}; {e}")),
                }
            }
        };
        if target.scheme != "https" {
            return Ok(Dialed {
                io: Box::new(tcp),
                alpn: None,
                verify_fallback: None,
            });
        }
        if !self.verify_tls {
            let tls = tls_connect(tcp, &target.host, &self.tls.insecure).await?;
            let alpn = alpn_of(&tls);
            return Ok(Dialed {
                io: Box::new(tls),
                alpn,
                verify_fallback: None,
            });
        }
        match tls_connect(tcp, &target.host, &self.tls.verifying).await {
            Ok(tls) => {
                let alpn = alpn_of(&tls);
                Ok(Dialed {
                    io: Box::new(tls),
                    alpn,
                    verify_fallback: None,
                })
            }
            Err(reason) => {
                // Fail open: verification is attempted first, and a certificate error is
                // answered by one retry without it, with the reason recorded on the flow.
                let tcp = connect_any(&addrs).await?;
                let tls = tls_connect(tcp, &target.host, &self.tls.insecure).await?;
                let alpn = alpn_of(&tls);
                Ok(Dialed {
                    io: Box::new(tls),
                    alpn,
                    verify_fallback: Some(reason),
                })
            }
        }
    }

    /// Resolves a host once and remembers the answer for a while.
    ///
    /// IPv4 addresses are returned first on purpose: a host with an AAAA record but no working
    /// IPv6 route would otherwise burn a full connect timeout on every single request.
    async fn resolve(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        if let Some(addrs) = self.cached_addrs(host) {
            return if addrs.is_empty() {
                Err(format!("cannot resolve {host} (temporary DNS failure)"))
            } else {
                Ok(addrs)
            };
        }

        let mut last = String::from("no address returned");
        // getaddrinfo reports EAI_AGAIN for transient resolver hiccups; one quick retry clears most.
        for attempt in 0..2 {
            match tokio::net::lookup_host((host, port)).await {
                Ok(iter) => {
                    let mut addrs: Vec<SocketAddr> = iter.collect();
                    if addrs.is_empty() {
                        break;
                    }
                    util::prefer_ipv4(&mut addrs);
                    addrs.dedup();
                    self.remember(host, &addrs, DNS_TTL);
                    return Ok(addrs);
                }
                Err(e) => {
                    last = e.to_string();
                    if attempt == 0 {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                    }
                }
            }
        }
        self.remember(host, &[], DNS_FAILURE_TTL);
        Err(format!("cannot resolve {host}: {last}"))
    }

    /// Resolves again, bypassing whatever the cache holds.
    async fn refresh(&self, host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
        self.forget(host);
        self.resolve(host, port).await
    }

    fn forget(&self, host: &str) {
        self.dns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(host);
    }

    fn cached_addrs(&self, host: &str) -> Option<Vec<SocketAddr>> {
        let dns = self.dns.lock().unwrap_or_else(|e| e.into_inner());
        let (at, addrs) = dns.get(host)?;
        let ttl = if addrs.is_empty() {
            DNS_FAILURE_TTL
        } else {
            DNS_TTL
        };
        (at.elapsed() < ttl).then(|| addrs.clone())
    }

    fn remember(&self, host: &str, addrs: &[SocketAddr], ttl: Duration) {
        let mut dns = self.dns.lock().unwrap_or_else(|e| e.into_inner());
        if dns.len() > 512 {
            dns.retain(|_, (at, _)| at.elapsed() < ttl);
        }
        dns.insert(
            host.to_string(),
            (std::time::Instant::now(), addrs.to_vec()),
        );
    }
}

/// Connects to the first address that answers; each attempt is bounded so one black hole cannot
/// eat the whole header timeout.
async fn connect_any(addrs: &[SocketAddr]) -> Result<TcpStream, String> {
    let mut last = String::from("no address to connect to");
    for addr in addrs {
        match timeout(ADDR_CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => {
                stream.set_nodelay(true).ok();
                return Ok(stream);
            }
            Ok(Err(e)) => last = format!("{addr}: {e}"),
            Err(_) => last = format!("{addr}: connect timed out"),
        }
    }
    Err(last)
}

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

struct Dialed {
    io: Box<dyn Io>,
    alpn: Option<String>,
    verify_fallback: Option<String>,
}

async fn tls_connect(
    tcp: TcpStream,
    host: &str,
    config: &Arc<rustls::ClientConfig>,
) -> Result<TlsStream<TcpStream>, String> {
    let server_name = ServerName::try_from(host.to_string()).map_err(|e| e.to_string())?;
    TlsConnector::from(config.clone())
        .connect(server_name, tcp)
        .await
        .map_err(|e| tls_error(&e))
}

fn tls_error(err: &io::Error) -> String {
    if let Some(inner) = err.get_ref() {
        if let Some(rustls_err) = inner.downcast_ref::<rustls::Error>() {
            return rustls_err.to_string();
        }
    }
    err.to_string()
}

fn alpn_of(tls: &TlsStream<TcpStream>) -> Option<String> {
    tls.get_ref()
        .1
        .alpn_protocol()
        .map(|protocol| String::from_utf8_lossy(protocol).into_owned())
}

fn destination(req: &Request<Incoming>, default_port: u16) -> Option<(String, u16)> {
    if let Some(authority) = req.uri().authority() {
        return Some(util::split_host_port(authority.as_str(), default_port));
    }
    let host = req.headers().get(header::HOST)?.to_str().ok()?;
    Some(util::split_host_port(host, default_port))
}

fn header_str(headers: &HeaderMap, name: HeaderName) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
}

fn new_flow(
    target: &Target,
    method: &Method,
    kind: Kind,
    started_at: u64,
    client_ip: &str,
    req_body: Option<BodyView>,
    req_headers: Vec<util::HeaderPair>,
) -> Flow {
    Flow {
        summary: FlowSummary {
            id: 0,
            seq: 0,
            scheme: target.scheme.to_string(),
            method: method.as_str().to_string(),
            url: target.url.clone(),
            host: target.host.clone(),
            port: target.port,
            path: target.path.clone(),
            status: None,
            status_text: None,
            kind,
            content_type: None,
            size: 0,
            duration_ms: None,
            started_at,
            ended_at: None,
            error: None,
            open: true,
            sse_count: None,
            ws_count: None,
            req_bytes: req_body.as_ref().map(|body| body.size),
            res_bytes: None,
        },
        http_version: "1.1".to_string(),
        req_headers,
        res_headers: Vec::new(),
        req_body,
        res_body: None,
        sse_frames: Vec::new(),
        ws_frames: Vec::new(),
        ttfb_ms: None,
        alpn: None,
        client_ip: Some(client_ip.to_string()),
        cert: None,
        notes: Vec::new(),
    }
}

fn tunnel_flow(authority: &str, host: &str, port: u16, client_ip: &str) -> Flow {
    Flow {
        summary: FlowSummary {
            id: 0,
            seq: 0,
            scheme: "https".to_string(),
            method: "CONNECT".to_string(),
            url: format!("https://{authority}"),
            host: host.to_string(),
            port,
            path: String::new(),
            status: Some(200),
            status_text: Some("Connection Established".to_string()),
            kind: Kind::Tunnel,
            content_type: None,
            size: 0,
            duration_ms: None,
            started_at: util::now_ms(),
            ended_at: None,
            error: None,
            open: true,
            sse_count: None,
            ws_count: None,
            req_bytes: None,
            res_bytes: None,
        },
        http_version: "1.1".to_string(),
        req_headers: Vec::new(),
        res_headers: Vec::new(),
        req_body: None,
        res_body: None,
        sse_frames: Vec::new(),
        ws_frames: Vec::new(),
        ttfb_ms: None,
        alpn: None,
        client_ip: Some(client_ip.to_string()),
        cert: None,
        notes: vec![Note::TunnelNotTls],
    }
}

fn text_response(status: StatusCode, message: &str) -> Response<api::Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(api::body_from(Bytes::from(message.to_string())))
        .unwrap()
}

/// Copies bytes one way until EOF, returning how many were forwarded.
async fn pipe<R, W>(reader: &mut R, writer: &mut W) -> u64
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 32 * 1024];
    let mut total = 0u64;
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n as u64;
                if writer.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = writer.shutdown().await;
    total
}

/// Like [`pipe`], but hands every chunk to a tap (WebSocket frames).
async fn pipe_tapped<R, W>(reader: &mut R, writer: &mut W, mut on_data: impl FnMut(&[u8])) -> u64
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = vec![0u8; 32 * 1024];
    let mut total = 0u64;
    loop {
        match reader.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                total += n as u64;
                on_data(&buf[..n]);
                if writer.write_all(&buf[..n]).await.is_err() {
                    break;
                }
            }
        }
    }
    let _ = writer.shutdown().await;
    total
}

/// A stream over the response channel; the pump task does the reading.
struct ChannelStream {
    rx: mpsc::Receiver<Result<Frame<Bytes>, io::Error>>,
}

impl Stream for ChannelStream {
    type Item = Result<Frame<Bytes>, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().rx.poll_recv(cx)
    }
}

/// Reads bytes from `inner`, serving `prefix` first (used to replay the peeked CONNECT byte).
struct PrefixedIo<S> {
    prefix: BytesMut,
    inner: S,
}

impl<S> PrefixedIo<S> {
    fn new(prefix: BytesMut, inner: S) -> Self {
        PrefixedIo { prefix, inner }
    }

    fn into_inner(self) -> S {
        self.inner
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedIo<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if !this.prefix.is_empty() {
            let take = this.prefix.len().min(buf.remaining());
            let chunk = this.prefix.split_to(take);
            buf.put_slice(&chunk);
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedIo<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Captures a response body: keeps a bounded copy, and parses SSE/NDJSON frames live.
struct BodyTap {
    kind: Kind,
    content_type: Option<String>,
    encoding: Option<String>,
    max_body: usize,
    chunks: Vec<Vec<u8>>,
    captured: usize,
    total: u64,
    dropped: u64,
    sse: Option<SseParser>,
    ndjson: Option<NdjsonParser>,
}

impl BodyTap {
    fn new(
        kind: Kind,
        content_type: Option<String>,
        encoding: Option<String>,
        max_body: usize,
    ) -> BodyTap {
        BodyTap {
            kind,
            content_type,
            encoding,
            max_body,
            chunks: Vec::new(),
            captured: 0,
            total: 0,
            dropped: 0,
            sse: (kind == Kind::Sse).then(SseParser::new),
            ndjson: (kind == Kind::Ndjson).then(NdjsonParser::new),
        }
    }

    fn data(&mut self, data: &[u8], store: &Store, flow_id: Option<u64>) {
        self.total += data.len() as u64;
        let room = self.max_body.saturating_sub(self.captured);
        if room > 0 {
            let take = data.len().min(room);
            self.chunks.push(data[..take].to_vec());
            self.captured += take;
        }
        if data.len() > room {
            self.dropped += (data.len() - room) as u64;
        }
        let Some(id) = flow_id else { return };
        match self.kind {
            Kind::Sse => {
                if let Some(parser) = self.sse.as_mut() {
                    let mut frames = Vec::new();
                    parser.feed(data, &mut |frame| frames.push(frame));
                    for frame in frames {
                        store.add_sse(id, frame);
                    }
                }
            }
            Kind::Ndjson => {
                if let Some(parser) = self.ndjson.as_mut() {
                    let mut frames = Vec::new();
                    parser.feed(data, &mut |frame| frames.push(frame));
                    for frame in frames {
                        store.add_sse(id, frame);
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(self) -> (Option<BodyView>, u64, u64) {
        let view = util::build_body(
            &self.chunks,
            self.total,
            self.dropped,
            self.content_type.as_deref(),
            self.encoding.as_deref(),
            self.max_body,
        );
        (view, self.total, self.dropped)
    }
}
