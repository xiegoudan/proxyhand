//! HAR 1.2 export, so captured flows can be replayed or opened in other tools.

use serde_json::{Value, json};

use crate::store::{Flow, Note};
use crate::util::HeaderPair;

fn headers_json(headers: &[HeaderPair]) -> Value {
    Value::Array(
        headers
            .iter()
            .map(|h| json!({ "name": h.name, "value": h.value }))
            .collect(),
    )
}

fn query_json(url: &str) -> Value {
    let Some((_, query)) = url.split_once('?') else {
        return Value::Array(Vec::new());
    };
    Value::Array(
        url::form_urlencoded::parse(query.as_bytes())
            .map(|(name, value)| json!({ "name": name.into_owned(), "value": value.into_owned() }))
            .collect(),
    )
}

pub fn har(flows: &[Flow], version: &str) -> Value {
    let mut entries = Vec::with_capacity(flows.len());
    for flow in flows {
        let s = &flow.summary;
        let duration = s.duration_ms.unwrap_or_else(|| {
            s.ended_at
                .unwrap_or(s.started_at)
                .saturating_sub(s.started_at)
        });
        let wait = flow.ttfb_ms.unwrap_or(duration);
        let receive = duration.saturating_sub(wait);
        let started = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(s.started_at as i64)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_else(|| "1970-01-01T00:00:00.000Z".to_string());

        let mut request = json!({
            "method": s.method,
            "url": s.url,
            "httpVersion": format!("HTTP/{}", flow.http_version),
            "cookies": [],
            "headers": headers_json(&flow.req_headers),
            "queryString": query_json(&s.url),
            "headersSize": -1,
            "bodySize": s.req_bytes.unwrap_or(0),
        });
        if let Some(body) = &flow.req_body {
            if let Some(text) = &body.text {
                request["postData"] = json!({
                    "mimeType": body.mime.clone().unwrap_or_else(|| "application/octet-stream".into()),
                    "text": text,
                });
            }
        }

        let mut content = json!({
            "size": s.size,
            "mimeType": s.content_type.clone().unwrap_or_default(),
        });
        if let Some(body) = &flow.res_body {
            if let Some(text) = &body.text {
                content["text"] = json!(text);
            } else if let Some(b64) = &body.base64 {
                content["text"] = json!(b64);
                content["encoding"] = json!("base64");
            }
        }
        let redirect = flow
            .res_headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case("location"))
            .map(|h| h.value.clone())
            .unwrap_or_default();

        let mut entry = json!({
            "startedDateTime": started,
            "time": duration,
            "request": request,
            "response": {
                "status": s.status.unwrap_or(0),
                "statusText": s.status_text.clone().unwrap_or_default(),
                "httpVersion": format!("HTTP/{}", flow.http_version),
                "cookies": [],
                "headers": headers_json(&flow.res_headers),
                "content": content,
                "redirectURL": redirect,
                "headersSize": -1,
                "bodySize": s.res_bytes.unwrap_or(s.size),
            },
            "cache": {},
            "timings": { "send": 0, "wait": wait, "receive": receive },
            "_kind": s.kind.as_str(),
        });
        if flow.notes.iter().any(|n| matches!(n, Note::TlsFallback(_))) {
            entry["_insecureTls"] = json!(true);
        }
        entries.push(entry);
    }

    json!({
        "log": {
            "version": "1.2",
            "creator": { "name": "proxyhand", "version": version },
            "entries": entries,
        }
    })
}
