//! A demo upstream for trying the proxy without touching the outside world:
//!
//! ```bash
//! cargo run --release --example demo-target   # http://127.0.0.1:9911
//! curl -x http://127.0.0.1:7747 http://127.0.0.1:9911/api/orders
//! curl -x http://127.0.0.1:7747 -N http://127.0.0.1:9911/sse
//! ```
//!
//! Endpoints: `/api/orders` (JSON), `/sse` (server-sent events), `/echo` (echoes method and body),
//! `/gzip` (compressed JSON), `/ws` (WebSocket echo), `/slow` (2s delay) and anything else → 404.

use std::convert::Infallible;
use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use flate2::Compression;
use flate2::write::GzEncoder;
use http::header;
use http::{Request, Response, StatusCode};
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper::server::conn::http1 as http1_server;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::handshake::derive_accept_key;
use tokio_tungstenite::tungstenite::protocol::{Message, Role};

type Body = http_body_util::combinators::BoxBody<Bytes, Infallible>;

fn bytes(body: impl Into<Bytes>) -> Body {
    Full::new(body.into())
        .map_err(|never: Infallible| match never {})
        .boxed()
}

fn reply(status: StatusCode, content_type: &str, body: impl Into<Bytes>) -> Response<Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .body(bytes(body))
        .unwrap()
}

fn json(body: serde_json::Value) -> Response<Body> {
    reply(StatusCode::OK, "application/json", body.to_string())
}

async fn handle(mut req: Request<Incoming>) -> Result<Response<Body>, Infallible> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let response = match (method.as_str(), path.as_str()) {
        ("GET", "/api/orders") => json(serde_json::json!({
            "orders": [
                { "id": 1001, "total": 42.5, "currency": "EUR", "items": 3 },
                { "id": 1002, "total": 7.0, "currency": "EUR", "items": 1 }
            ]
        })),
        ("GET", "/sse") => {
            // One event every 400ms, then a long quiet tail: the interesting case for a proxy.
            let stream = futures_util::stream::unfold(0u32, |n| async move {
                tokio::time::sleep(Duration::from_millis(400)).await;
                if n >= 3 {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    return None;
                }
                let frame = format!("id: {}\nevent: tick\ndata: {{\"n\":{}}}\n\n", n + 1, n + 1);
                Some((Ok::<_, Infallible>(Frame::data(Bytes::from(frame))), n + 1))
            });
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .header(header::CACHE_CONTROL, "no-cache")
                .body(StreamBody::new(stream).boxed())
                .unwrap()
        }
        ("GET", "/gzip") => {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            let _ = encoder
                .write_all(b"{\"gzip\":true,\"note\":\"the proxy decodes this for the flow\"}");
            let compressed = encoder.finish().unwrap_or_default();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_ENCODING, "gzip")
                .body(bytes(compressed))
                .unwrap()
        }
        ("GET", "/slow") => {
            tokio::time::sleep(Duration::from_secs(2)).await;
            json(serde_json::json!({ "slow": true }))
        }
        ("GET", "/ws") => return Ok(upgrade_ws(&mut req)),
        ("POST" | "PUT" | "PATCH" | "DELETE" | "GET", "/echo") => {
            let collected = req
                .into_body()
                .collect()
                .await
                .map(|c| c.to_bytes())
                .unwrap_or_default();
            json(serde_json::json!({
                "method": method.as_str(),
                "body": String::from_utf8_lossy(&collected),
            }))
        }
        _ => reply(StatusCode::NOT_FOUND, "text/plain", "nope"),
    };
    Ok(response)
}

fn upgrade_ws(req: &mut Request<Incoming>) -> Response<Body> {
    let key = req
        .headers()
        .get("sec-websocket-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    let accept = derive_accept_key(key.as_bytes());
    let upgrade = hyper::upgrade::on(req);
    tokio::spawn(async move {
        let Ok(upgraded) = upgrade.await else { return };
        let mut ws =
            WebSocketStream::from_raw_socket(TokioIo::new(upgraded), Role::Server, None).await;
        while let Some(Ok(message)) = futures_util::StreamExt::next(&mut ws).await {
            let echo = match message {
                Message::Text(text) => Message::Text(format!("echo:{text}").into()),
                Message::Binary(payload) => Message::Binary(payload),
                Message::Ping(payload) => Message::Pong(payload),
                Message::Close(_) => break,
                other => other,
            };
            if futures_util::SinkExt::send(&mut ws, echo).await.is_err() {
                break;
            }
        }
    });
    Response::builder()
        .status(StatusCode::SWITCHING_PROTOCOLS)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header("sec-websocket-accept", accept)
        .body(bytes(Bytes::new()))
        .unwrap()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(9911);
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port))).await?;
    println!("demo target on http://127.0.0.1:{port}  (/api/orders /sse /gzip /echo /slow /ws)");
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            let service = service_fn(handle);
            let _ = http1_server::Builder::new()
                .serve_connection(TokioIo::new(stream), service)
                .with_upgrades()
                .await;
        });
    }
}
