//! End-to-end tests that drive a real proxy instance with raw HTTP/1.1 clients.

mod common;

use std::time::{Duration, Instant};

use common::{
    api_get, api_get_json, connect, https_get_via_proxy, proxy_get, read_response_head, request,
    start_http_upstream, start_https_upstream, start_proxy, wait_for_flow,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn find_header<'a>(headers: &'a Value, name: &str) -> Option<&'a str> {
    headers.as_array()?.iter().find_map(|header| {
        (header["name"].as_str()? == name).then(|| header["value"].as_str().unwrap_or_default())
    })
}

fn body_text(detail: &Value) -> Option<&str> {
    detail["resBody"]["text"].as_str()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_get_is_captured_with_headers_and_body() {
    let proxy = start_proxy(true, &[], true).await;
    let upstream = start_http_upstream().await;
    let url = format!("http://{}/hello", upstream.addr);

    let response = proxy_get(proxy.addr, &url).await;
    assert_eq!(response.status, 200, "body: {}", response.text());
    assert_eq!(response.text(), "hello from upstream");
    assert_eq!(
        response.header("content-type"),
        Some("text/plain; charset=utf-8")
    );
    assert_eq!(response.header("x-upstream"), Some("proxyhand-test"));

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["endedAt"].is_number() && flow["open"] == false
    })
    .await;

    assert_eq!(detail["kind"], "http");
    assert_eq!(detail["status"], 200);
    assert_eq!(detail["contentType"], "text/plain; charset=utf-8");
    assert_eq!(detail["size"], response.body.len() as u64);
    assert_eq!(detail["scheme"], "http");
    // The UI renders "HTTP/{httpVersion}", so the stored version must not carry the prefix itself.
    assert_eq!(
        detail["httpVersion"], "1.1",
        "no double \"HTTP/HTTP/\" prefix"
    );
    // A finished flow must have a duration: the close patch carries its own ended_at.
    assert!(
        detail["durationMs"].is_number(),
        "finished flow must report a duration, got {}",
        detail["durationMs"]
    );

    let upstream_authority = upstream.addr.to_string();
    assert_eq!(
        find_header(&detail["reqHeaders"], "host"),
        Some(upstream_authority.as_str())
    );
    assert_eq!(
        find_header(&detail["resHeaders"], "x-upstream"),
        Some("proxyhand-test")
    );

    assert!(
        detail["reqBody"].is_null(),
        "a bodyless GET gets no request body view"
    );
    assert_eq!(body_text(&detail), Some("hello from upstream"));
    assert_eq!(detail["resBody"]["mime"], "text/plain");
    assert_eq!(
        detail["resBody"]["size"],
        response.body.len() as u64,
        "body view should report the transferred size"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_http_post_body_is_captured() {
    let proxy = start_proxy(true, &[], true).await;
    let upstream = start_http_upstream().await;
    let url = format!("http://{}/echo", upstream.addr);

    let response = request(
        proxy.addr,
        "POST",
        &url,
        &upstream.addr.to_string(),
        &[("content-type", "text/plain")],
        b"payload",
    )
    .await;
    assert_eq!(response.status, 200, "body: {}", response.text());
    let echoed = response.json();
    assert_eq!(echoed["method"], "POST");
    assert_eq!(echoed["body"], "payload");

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["endedAt"].is_number() && flow["open"] == false
    })
    .await;
    assert_eq!(detail["method"], "POST");
    assert_eq!(detail["reqBody"]["text"], "payload");
    assert_eq!(detail["reqBody"]["mime"], "text/plain");
    assert!(body_text(&detail).is_some_and(|body| body.contains("payload")));
}

#[tokio::test(flavor = "multi_thread")]
async fn sse_upstream_records_three_frames() {
    let proxy = start_proxy(true, &[], true).await;
    let upstream = start_http_upstream().await;
    let url = format!("http://{}/sse", upstream.addr);

    let response = proxy_get(proxy.addr, &url).await;
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("text/event-stream"));
    assert!(response.text().contains("event: task"));
    assert!(response.text().contains("id: 7"));
    assert!(response.text().contains("data: [done]"));

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["sseFrames"]
            .as_array()
            .is_some_and(|frames| frames.len() == 3)
            && flow["open"] == false
    })
    .await;

    assert_eq!(detail["kind"], "sse");
    assert_eq!(detail["sseCount"], 3);
    let frames = detail["sseFrames"].as_array().expect("sse frames");
    assert_eq!(frames[0]["event"], "task");
    assert_eq!(frames[0]["data"], "{\"n\":1}");
    assert_eq!(frames[1]["id"], "7");
    assert_eq!(frames[1]["data"], "{\"n\":2}");
    assert_eq!(frames[2]["data"], "[done]");
}

#[tokio::test(flavor = "multi_thread")]
async fn idle_sse_stream_stays_open_past_one_second() {
    let proxy = start_proxy(true, &[], true).await;
    let upstream = start_http_upstream().await;
    let authority = upstream.addr.to_string();
    let request_text = format!(
        "GET http://{authority}/idle-sse HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    );

    let mut client = connect(proxy.addr).await;
    client
        .write_all(request_text.as_bytes())
        .await
        .expect("write idle SSE request");

    let head = tokio::time::timeout(Duration::from_secs(3), read_response_head(&mut client))
        .await
        .expect("idle SSE response headers timed out")
        .expect("read idle SSE headers");
    assert_eq!(head.status, 200);
    assert_eq!(
        head.headers
            .iter()
            .find(|(name, _)| name == "content-type")
            .map(|(_, value)| value.as_str()),
        Some("text/event-stream")
    );

    let started = Instant::now();
    let mut body = head.leftover;
    while !contains(&body, b": open") {
        let mut chunk = [0u8; 1024];
        let read = tokio::time::timeout(Duration::from_secs(3), client.read(&mut chunk))
            .await
            .expect("first SSE comment timed out")
            .expect("read first SSE comment");
        assert!(read > 0, "stream closed before the comment arrived");
        body.extend_from_slice(&chunk[..read]);
    }
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the comment should arrive promptly"
    );

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["kind"] == "sse" && flow["sseCount"] == 1
    })
    .await;
    assert_eq!(
        detail["open"].as_bool(),
        Some(true),
        "idle stream should still be open"
    );
    assert_eq!(
        detail["sseFrames"][0]["comment"], "open",
        "comment-only frames are part of the stream"
    );

    // The body idle timeout exists for requests that stall, but it must never
    // apply to a stream that is simply quiet.
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let mut probe = [0u8; 64];
    match tokio::time::timeout(Duration::from_millis(300), client.read(&mut probe)).await {
        Err(_) => {}
        Ok(Ok(0)) => panic!("proxy closed an idle SSE stream"),
        Ok(Ok(read)) => panic!("unexpected extra SSE bytes while idle: {read}"),
        Ok(Err(error)) => panic!("idle SSE stream failed: {error}"),
    }
    assert!(started.elapsed() >= Duration::from_secs(1));

    let still_open = api_get_json(proxy.addr, "/api/flows/1").await.1;
    assert_eq!(
        still_open["open"].as_bool(),
        Some(true),
        "flow must remain open while the upstream stream is idle"
    );
}

#[test]
fn body_idle_timeout_is_120s_and_not_applied_to_streams() {
    // Waiting for the real timeout would add two minutes to the suite, so this
    // asserts the configured constant and the branch that bypasses it.
    let source = include_str!("../src/proxy.rs");
    let normalized = source.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        normalized.contains("const BODY_IDLE_TIMEOUT: Duration = Duration::from_secs(120);"),
        "body idle timeout should stay at 120s"
    );
    assert!(
        normalized.contains("let streaming = matches!(kind, Kind::Sse | Kind::Ndjson);"),
        "SSE/NDJSON must be classified as streaming"
    );
    assert!(
        normalized.contains("let next = if streaming { stream.next().await"),
        "streaming bodies must not be wrapped in the idle timeout"
    );
    assert!(
        normalized.contains("match timeout(BODY_IDLE_TIMEOUT, stream.next()).await"),
        "non-streaming bodies must use the idle timeout"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn flow_listing_order_limit_and_query_filters() {
    let proxy = start_proxy(true, &[], true).await;
    let upstream = start_http_upstream().await;
    let base = format!("http://{}", upstream.addr);

    let first = proxy_get(proxy.addr, &format!("{base}/hello")).await;
    assert_eq!(first.status, 200);
    let second = proxy_get(proxy.addr, &format!("{base}/sse")).await;
    assert_eq!(second.status, 200);
    let third = request(
        proxy.addr,
        "POST",
        &format!("{base}/echo"),
        &upstream.addr.to_string(),
        &[("content-type", "text/plain")],
        b"third",
    )
    .await;
    assert_eq!(third.status, 200);

    for id in 1..=3 {
        wait_for_flow(proxy.addr, id, |flow| {
            flow["endedAt"].is_number() && flow["open"] == false
        })
        .await;
    }

    let (_, all) = api_get_json(proxy.addr, "/api/flows").await;
    assert_eq!(all["total"], 3);
    let ids: Vec<u64> = all["flows"]
        .as_array()
        .expect("flows array")
        .iter()
        .map(|flow| flow["id"].as_u64().expect("flow id"))
        .collect();
    assert_eq!(
        ids,
        vec![1, 2, 3],
        "a missing limit must still return every flow"
    );

    let (_, descending) = api_get_json(proxy.addr, "/api/flows?order=desc").await;
    let ids: Vec<u64> = descending["flows"]
        .as_array()
        .expect("flows array")
        .iter()
        .map(|flow| flow["id"].as_u64().expect("flow id"))
        .collect();
    assert_eq!(ids, vec![3, 2, 1]);

    let (_, limited) = api_get_json(proxy.addr, "/api/flows?limit=2").await;
    assert_eq!(limited["flows"].as_array().expect("flows array").len(), 2);
    assert_eq!(limited["total"], 3);

    let (_, searched) = api_get_json(proxy.addr, "/api/flows?q=hello").await;
    let paths: Vec<&str> = searched["flows"]
        .as_array()
        .expect("flows array")
        .iter()
        .filter_map(|flow| flow["path"].as_str())
        .collect();
    assert_eq!(paths, vec!["/hello"]);

    let (_, missed) = api_get_json(proxy.addr, "/api/flows?q=nothing-matches").await;
    assert_eq!(missed["flows"].as_array().expect("flows array").len(), 0);
    assert_eq!(missed["total"], 0);
}

#[tokio::test(flavor = "multi_thread")]
async fn loop_guard_answers_508() {
    // The UI normally intercepts requests to the proxy port, so disable it to
    // exercise the loop guard itself.
    let proxy = start_proxy(true, &[], false).await;
    let authority = format!("127.0.0.1:{}", proxy.addr.port());
    let url = format!("http://{authority}/blocked");

    let response = request(
        proxy.addr,
        "GET",
        &url,
        &authority,
        &[("x-proxyhand-loop", "1")],
        &[],
    )
    .await;
    assert_eq!(response.status, 508, "body: {}", response.text());
    assert!(response.text().contains("loop detected"));
    assert_eq!(
        proxy
            .store
            .list(&proxyhand::store::ListQuery {
                limit: 10,
                ..Default::default()
            })
            .0,
        0,
        "the loop guard must answer before creating a flow"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn https_upstream_without_trusted_ca_warns_but_still_works() {
    let upstream = start_https_upstream().await;
    // Verification stays enabled so the proxy can notice the unknown CA and
    // fail open; the test CA is deliberately not in UpstreamTls.
    let proxy = start_proxy(true, &[], true).await;
    let authority = format!("localhost:{}", upstream.addr.port());

    let response = https_get_via_proxy(proxy.addr, &proxy.ca_pem, &authority, "/secure").await;
    assert_eq!(response.status, 200, "body: {}", response.text());
    assert_eq!(response.text(), "secure:/secure");

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["notes"]
            .as_array()
            .is_some_and(|notes| !notes.is_empty())
            && flow["endedAt"].is_number()
    })
    .await;
    assert_eq!(detail["scheme"], "https");
    assert_eq!(detail["host"], "localhost");
    assert_eq!(detail["status"], 200);
    let notes = detail["notes"].as_array().expect("notes array");
    let joined = notes
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("certificate was not verified"),
        "missing verification note: {joined}"
    );
    assert!(joined.contains("--upstream-ca"), "missing hint: {joined}");
}

#[tokio::test(flavor = "multi_thread")]
async fn https_upstream_with_trusted_ca_has_no_notes() {
    let upstream = start_https_upstream().await;
    let proxy = start_proxy(true, std::slice::from_ref(&upstream.ca_pem), true).await;
    let authority = format!("localhost:{}", upstream.addr.port());

    let response = https_get_via_proxy(proxy.addr, &proxy.ca_pem, &authority, "/secure").await;
    assert_eq!(response.status, 200, "body: {}", response.text());
    assert_eq!(response.text(), "secure:/secure");

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["endedAt"].is_number() && flow["open"] == false
    })
    .await;
    assert_eq!(detail["status"], 200);
    assert!(
        detail.get("notes").is_none(),
        "a trusted upstream must not produce notes: {}",
        detail["notes"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn https_insecure_mode_has_no_verification_note() {
    let upstream = start_https_upstream().await;
    // verify_tls=false dials with the insecure config directly; no fallback
    // happened, so there is nothing to warn about.
    let proxy = start_proxy(false, &[], true).await;
    let authority = format!("localhost:{}", upstream.addr.port());

    let response = https_get_via_proxy(proxy.addr, &proxy.ca_pem, &authority, "/secure").await;
    assert_eq!(response.status, 200, "body: {}", response.text());

    let detail = wait_for_flow(proxy.addr, 1, |flow| {
        flow["endedAt"].is_number() && flow["open"] == false
    })
    .await;
    assert!(
        detail.get("notes").is_none(),
        "insecure mode should not add a verification note: {}",
        detail["notes"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn har_export_endpoint_returns_entries_and_filename() {
    let proxy = start_proxy(true, &[], true).await;
    let upstream = start_http_upstream().await;
    let url = format!("http://{}/hello", upstream.addr);
    assert_eq!(proxy_get(proxy.addr, &url).await.status, 200);
    wait_for_flow(proxy.addr, 1, |flow| {
        flow["endedAt"].is_number() && flow["open"] == false
    })
    .await;

    let response = api_get(proxy.addr, "/api/export/har").await;
    assert_eq!(response.status, 200);
    assert!(
        response
            .header("content-type")
            .is_some_and(|value| value.starts_with("application/json"))
    );
    let disposition = response
        .header("content-disposition")
        .expect("Content-Disposition header");
    assert!(
        disposition.starts_with("attachment; filename=\"proxyhand-"),
        "unexpected disposition: {disposition}"
    );
    assert!(
        disposition.ends_with(".har\""),
        "unexpected disposition: {disposition}"
    );

    let value = response.json();
    let entries = value["log"]["entries"].as_array().expect("HAR entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["request"]["httpVersion"], "HTTP/1.1");
    assert_eq!(entries[0]["response"]["httpVersion"], "HTTP/1.1");
    assert_eq!(entries[0]["response"]["status"], 200);
    assert!(
        entries[0]["startedDateTime"]
            .as_str()
            .is_some_and(|value| value.ends_with('Z') && value.contains('T'))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn ui_answers_on_any_published_port() {
    let proxy = start_proxy(true, &[], true).await;

    // `docker run -p 8899:7747` makes the browser type a port the proxy does not
    // listen on. The request is a plain path addressed to a hostname we own, so
    // it must land on the UI instead of being forwarded to "127.0.0.1:65535".
    let response = request(
        proxy.addr,
        "GET",
        "/",
        "127.0.0.1:65535",
        &[("accept", "text/html")],
        &[],
    )
    .await;

    assert_eq!(response.status, 200, "body: {}", response.text());
    assert!(
        response.text().to_lowercase().contains("<!doctype html"),
        "expected the embedded UI, got: {}",
        &response.text()[..response.text().len().min(200)]
    );

    // A browser hit is not a captured flow.
    assert_eq!(proxy.store.stats().total, 0);
}
