//! Replaying a captured request.
//!
//! The request is pushed back through proxyhand's own listener on loopback, so a replay goes down
//! exactly the same path as live traffic: TLS interception, capture, notes, the lot. Nothing here
//! talks to the upstream directly, which keeps one implementation of "send a request".

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::{HeaderName, HeaderValue, Method, Request};
use http_body_util::{BodyExt, Full};
use serde_json::Value;
use tokio::net::TcpStream;

use crate::store::Store;

/// Response headers are drained before we answer, so this only has to outlast the upstream.
const SEND_TIMEOUT: Duration = Duration::from_secs(60);
/// Long enough for the capture to close the flow, short enough not to hang the UI.
const CAPTURE_GRACE: Duration = Duration::from_secs(3);
const MAX_BODY: usize = 32 * 1024 * 1024;

pub struct Replay {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Replay {
    /// Reads `{ method, url, headers: [[name, value]], body, bodyBase64 }` from the UI.
    pub fn parse(input: &Value) -> Result<Self, String> {
        let method = input
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("GET")
            .parse::<Method>()
            .map_err(|_| "invalid method".to_string())?;

        let url = input
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let parsed = url
            .parse::<http::Uri>()
            .map_err(|_| format!("invalid url: {url}"))?;
        if parsed.scheme().is_none() || parsed.authority().is_none() {
            return Err(format!("url must be absolute (http:// or https://): {url}"));
        }

        let mut headers = Vec::new();
        if let Some(list) = input.get("headers").and_then(Value::as_array) {
            for entry in list {
                let pair = match entry {
                    Value::Array(items) if items.len() >= 2 => Some((
                        items[0].as_str().unwrap_or("").to_string(),
                        items[1].as_str().unwrap_or("").to_string(),
                    )),
                    Value::Object(map) => Some((
                        map.get("name")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        map.get("value")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                    )),
                    _ => None,
                };
                let Some((name, value)) = pair else { continue };
                if name.trim().is_empty() {
                    continue;
                }
                headers.push((name.trim().to_string(), value));
            }
        }

        let body = match input.get("bodyBase64").and_then(Value::as_str) {
            Some(encoded) if !encoded.is_empty() => decode_base64(encoded)?,
            _ => input
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes()
                .to_vec(),
        };
        if body.len() > MAX_BODY {
            return Err(format!("body larger than {MAX_BODY} bytes"));
        }

        Ok(Replay {
            method,
            url,
            headers,
            body,
        })
    }
}

/// Sends the replay through the proxy listening on `port` and returns the flow it created.
pub async fn send(store: Arc<Store>, port: u16, replay: Replay) -> Result<u64, String> {
    let before = store.last_id();

    let io = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| format!("cannot reach the proxy on 127.0.0.1:{port}: {e}"))?;
    io.set_nodelay(true).ok();
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
            .await
            .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let mut builder = Request::builder()
        .method(replay.method.clone())
        .uri(&replay.url)
        .version(http::Version::HTTP_11);
    for (name, value) in &replay.headers {
        let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) else {
            continue;
        };
        // Length is recomputed for the bytes actually sent.
        if name == http::header::CONTENT_LENGTH || name == http::header::HOST {
            continue;
        }
        builder = builder.header(name, value);
    }
    let request = builder
        .body(Full::new(Bytes::from(replay.body)))
        .map_err(|e| e.to_string())?;

    let response = tokio::time::timeout(SEND_TIMEOUT, sender.send_request(request))
        .await
        .map_err(|_| format!("no response within {}s", SEND_TIMEOUT.as_secs()))?
        .map_err(|e| e.to_string())?;

    // Draining the body is what lets the proxy finish and close the flow.
    let mut response = response;
    while let Some(frame) = response.body_mut().frame().await {
        if frame.is_err() {
            break;
        }
    }

    let id = before + 1;
    let deadline = tokio::time::Instant::now() + CAPTURE_GRACE;
    while tokio::time::Instant::now() < deadline {
        if let Some(flow) = store.get(id) {
            if !flow.summary.open {
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(40)).await;
    }

    if store.get(id).is_none() {
        return Err("the replay was sent but no flow was recorded".to_string());
    }
    Ok(id)
}

fn decode_base64(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input.trim())
        .map_err(|e| format!("invalid base64 body: {e}"))
}
