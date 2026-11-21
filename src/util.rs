//! Shared helpers: bodies, headers, connection targets and local-host detection.

use std::io::Read;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use http::uri::{Authority, Uri};
use http::{HeaderMap, HeaderValue};
use serde::Serialize;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HeaderPair {
    pub name: String,
    pub value: String,
}

/// Converts a [`HeaderMap`] (lowercase names, repeated values preserved) into wire pairs.
pub fn header_pairs(headers: &HeaderMap) -> Vec<HeaderPair> {
    headers
        .iter()
        .map(|(name, value)| HeaderPair {
            name: name.as_str().to_string(),
            value: String::from_utf8_lossy(value.as_bytes()).into_owned(),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyView {
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decoded_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encoding: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// Bytes that were never captured because the per-body limit was hit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped: Option<u64>,
}

/// Builds the body view the UI consumes: text when it is textual, base64 otherwise.
///
/// `raw_size` is the number of bytes actually transferred, `dropped` counts bytes that were not
/// captured, so the UI can say "truncated" instead of pretending the body is complete.
pub fn build_body(
    chunks: &[Vec<u8>],
    raw_size: u64,
    dropped: u64,
    content_type: Option<&str>,
    encoding: Option<&str>,
    max_body: usize,
) -> Option<BodyView> {
    let mime = content_type
        .and_then(|ct| ct.split(';').next())
        .map(|m| m.trim().to_string());
    if raw_size == 0 && chunks.is_empty() {
        return None;
    }
    let wire: Vec<u8> = chunks.concat();
    let size = raw_size.max(wire.len() as u64);
    let encoding = encoding
        .map(|e| e.trim().to_ascii_lowercase())
        .filter(|e| !e.is_empty());
    let encoding = encoding.filter(|e| e != "identity");

    let mut decoded_size = None;
    let mut body: Vec<u8> = wire.clone();
    if let Some(enc) = &encoding {
        if let Some(decoded) = decompress(enc, &wire) {
            decoded_size = Some(decoded.len() as u64);
            body = decoded;
        }
    }

    let truncated = body.len() > max_body;
    if truncated {
        body.truncate(max_body);
    }

    let textual = looks_textual(content_type, &body);
    let mut view = BodyView {
        size,
        decoded_size,
        mime: mime.clone(),
        encoding: encoding.clone(),
        text: None,
        base64: None,
        truncated,
        dropped: (dropped > 0).then_some(dropped),
    };
    if textual {
        let mut t = String::from_utf8_lossy(&body).into_owned();
        if truncated {
            t.push_str(&format!("\n… ({size} bytes total)"));
        }
        view.text = Some(t);
    } else {
        let mut b = base64::engine::general_purpose::STANDARD.encode(&body);
        if truncated {
            b.push('…');
        }
        view.base64 = Some(b);
    }
    Some(view)
}

/// HTML feature-detection from the TS implementation: any `<!doctype`, `<html` or `<head` counts.
fn looks_textual(content_type: Option<&str>, body: &[u8]) -> bool {
    if let Some(ct) = content_type {
        let ct = ct.to_ascii_lowercase();
        if ct.starts_with("text/")
            || ct.contains("json")
            || ct.contains("javascript")
            || ct.contains("xml")
            || ct.contains("graphql")
            || ct.contains("x-www-form-urlencoded")
            || ct.starts_with("multipart/form-data") && std::str::from_utf8(body).is_ok()
            || ct.contains("event-stream")
        {
            return true;
        }
    }
    let head: String = String::from_utf8_lossy(&body[..body.len().min(2048)]).to_ascii_lowercase();
    head.contains("<!doctype") || head.contains("<html") || head.contains("<head")
}

fn decompress(encoding: &str, data: &[u8]) -> Option<Vec<u8>> {
    match encoding {
        "gzip" | "x-gzip" => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(data)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        "deflate" => {
            let mut out = Vec::new();
            if flate2::read::DeflateDecoder::new(data)
                .read_to_end(&mut out)
                .is_ok()
            {
                return Some(out);
            }
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(data)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        "br" => {
            let mut out = Vec::new();
            brotli::Decompressor::new(data, 4096)
                .read_to_end(&mut out)
                .ok()?;
            Some(out)
        }
        _ => None,
    }
}

pub fn is_streaming(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else { return false };
    ct.to_ascii_lowercase()
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        == "text/event-stream"
}

pub fn is_ndjson(content_type: Option<&str>) -> bool {
    let Some(ct) = content_type else { return false };
    let ct = ct.to_ascii_lowercase();
    ct.split(';').next().unwrap_or("").trim().contains("ndjson")
}

/// Headers that describe a single connection hop and must not be forwarded.
pub fn hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "transfer-encoding"
            | "proxy-connection"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "upgrade"
    )
}

pub fn is_upgrade(headers: &HeaderMap) -> bool {
    headers
        .get("upgrade")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().eq_ignore_ascii_case("websocket"))
        .unwrap_or(false)
}

/// Where a request should be sent, derived from the request line and `Host` header.
#[derive(Debug, Clone)]
pub struct Target {
    pub scheme: &'static str,
    pub host: String,
    pub port: u16,
    /// Path + query, as sent on the wire.
    pub path: String,
    pub url: String,
    /// Host header the upstream should see (`host:port` unless the port is the default).
    pub authority: String,
    /// `host:port` always, for dialing.
    pub connect: String,
}

pub fn resolve_target(
    uri: &Uri,
    headers: &HeaderMap,
    is_tls: bool,
    default_authority: Option<&Authority>,
) -> Option<Target> {
    // Absolute-form request line (plain HTTP through the proxy).
    if let Some(scheme) = uri.scheme_str() {
        let host = uri.host()?.to_string();
        let scheme = if scheme.eq_ignore_ascii_case("https") {
            "https"
        } else {
            "http"
        };
        let port = uri
            .port_u16()
            .unwrap_or(if scheme == "https" { 443 } else { 80 });
        let path = uri
            .path_and_query()
            .map(|p| p.as_str())
            .unwrap_or("/")
            .to_string();
        return Some(Target {
            scheme,
            url: format!("{scheme}://{}:{}{}", bracket(&host), port, path),
            authority: authority_for(&host, port, scheme),
            connect: format!("{}:{}", bracket(&host), port),
            host,
            port,
            path,
        });
    }

    let (host, port) = match headers
        .get(http::header::HOST)
        .and_then(|v| v.to_str().ok())
    {
        Some(host) => split_host_port(host.trim(), if is_tls { 443 } else { 80 }),
        None => {
            let authority = default_authority?;
            split_host_port(authority.as_str(), if is_tls { 443 } else { 80 })
        }
    };
    let scheme = if is_tls { "https" } else { "http" };
    let path = uri
        .path_and_query()
        .map(|p| p.as_str())
        .unwrap_or("/")
        .to_string();
    Some(Target {
        scheme,
        url: format!("{scheme}://{}:{}{}", bracket(&host), port, path),
        authority: authority_for(&host, port, scheme),
        connect: format!("{}:{}", bracket(&host), port),
        host,
        port,
        path,
    })
}

pub fn bracket(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

fn authority_for(host: &str, port: u16, scheme: &str) -> String {
    let default = if scheme == "https" { 443 } else { 80 };
    if port == default {
        host.to_string()
    } else {
        format!("{}:{}", bracket(host), port)
    }
}

/// Sorts resolved addresses so IPv4 comes first.
///
/// A host that publishes an AAAA record but has no working IPv6 route would otherwise burn a
/// connect timeout on every request before falling back to IPv4.
pub fn prefer_ipv4(addrs: &mut [std::net::SocketAddr]) {
    addrs.sort_by_key(|addr| addr.is_ipv6());
}

/// Splits `host:port`, `[::1]:443`, `[::1]`, `example.com` into host + port.
pub fn split_host_port(value: &str, default_port: u16) -> (String, u16) {
    if let Some(rest) = value.strip_prefix('[') {
        if let Some((host, tail)) = rest.split_once(']') {
            let port = tail
                .strip_prefix(':')
                .and_then(|p| p.parse().ok())
                .unwrap_or(default_port);
            return (host.to_string(), port);
        }
    }
    match value.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) && !port.is_empty() => {
            (host.to_string(), port.parse().unwrap_or(default_port))
        }
        _ => (value.to_string(), default_port),
    }
}

pub fn header_value(v: &str) -> HeaderValue {
    HeaderValue::from_str(v).unwrap_or_else(|_| HeaderValue::from_static(""))
}

/// Hosts that mean "the UI lives here": loopback, the configured bind host, the machine's
/// hostname, every interface address, plus user supplied aliases.
pub fn local_hosts(bind_host: &str, extra: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = vec![
        "localhost".into(),
        "127.0.0.1".into(),
        "::1".into(),
        "0.0.0.0".into(),
        bind_host.to_string(),
    ];
    if let Ok(name) = hostname::get() {
        hosts.push(name.to_string_lossy().to_string());
    }
    if let Ok(addrs) = if_addrs::get_if_addrs() {
        for addr in addrs {
            if !addr.is_loopback() {
                hosts.push(addr.ip().to_string());
            }
        }
    }
    for e in extra {
        let e = e.trim();
        if !e.is_empty() {
            hosts.push(e.to_string());
        }
    }
    hosts.sort();
    hosts.dedup();
    hosts
}

pub fn host_matches(hosts: &[String], host: &str, port: u16, proxy_port: u16) -> bool {
    port == proxy_port && host_is_local(hosts, host)
}

/// Whether `host` names this machine, ignoring the port.
///
/// A browser pointed straight at the proxy sends an origin-form target, and the
/// port it used is whatever the user typed — with Docker that is the published
/// port (`-p 8899:7747`), not the port inside the container.
pub fn host_is_local(hosts: &[String], host: &str) -> bool {
    let host = host.trim_matches(|c| c == '[' || c == ']');
    hosts.iter().any(|h| h.eq_ignore_ascii_case(host))
}

pub fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        508 => "Loop Detected",
        _ => "",
    }
}
