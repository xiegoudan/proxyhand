//! Store ordering/filtering/eviction and HAR export tests.

use proxyhand::har;
use proxyhand::store::{Flow, FlowSummary, Kind, ListQuery, Note, Order, StatusFilter, Store};

fn flow(method: &str, path: &str, status: Option<u16>, kind: Kind, open: bool) -> Flow {
    Flow {
        summary: FlowSummary {
            id: 0,
            seq: 0,
            scheme: "http".to_string(),
            method: method.to_string(),
            url: format!("http://example.test{path}"),
            host: "example.test".to_string(),
            port: 80,
            path: path.to_string(),
            status,
            status_text: status.map(|code| {
                if code < 400 {
                    "OK".to_string()
                } else {
                    "Error".to_string()
                }
            }),
            kind,
            content_type: Some("text/plain".to_string()),
            size: 11,
            duration_ms: Some(3),
            started_at: 1_700_000_000_000,
            ended_at: Some(1_700_000_000_003),
            error: status
                .filter(|code| *code >= 400)
                .map(|_| "boom".to_string()),
            open,
            sse_count: None,
            ws_count: None,
            req_bytes: Some(0),
            res_bytes: Some(11),
        },
        http_version: "1.1".to_string(),
        req_headers: Vec::new(),
        res_headers: Vec::new(),
        req_body: None,
        res_body: None,
        sse_frames: Vec::new(),
        ws_frames: Vec::new(),
        ttfb_ms: Some(1),
        alpn: None,
        client_ip: None,
        cert: None,
        notes: Vec::new(),
    }
}

fn ids(rows: &[FlowSummary]) -> Vec<u64> {
    rows.iter().map(|row| row.id).collect()
}

fn all(store: &Store) -> (usize, Vec<FlowSummary>) {
    store.list(&ListQuery {
        limit: 100,
        ..ListQuery::default()
    })
}

#[test]
fn store_lists_flows_chronologically_by_default() {
    let store = Store::new(100, "test");
    for path in ["/one", "/two", "/three"] {
        store
            .create(flow("GET", path, Some(200), Kind::Http, false))
            .expect("flow should be recorded");
    }

    let (total, rows) = all(&store);
    assert_eq!(total, 3);
    assert_eq!(ids(&rows), vec![1, 2, 3]);
    assert_eq!(
        rows.iter().map(|row| row.path.as_str()).collect::<Vec<_>>(),
        vec!["/one", "/two", "/three"]
    );
}

#[test]
fn store_desc_reverses_the_page() {
    let store = Store::new(100, "test");
    for path in ["/one", "/two", "/three"] {
        store
            .create(flow("GET", path, Some(200), Kind::Http, false))
            .expect("flow should be recorded");
    }

    let (_, rows) = store.list(&ListQuery {
        limit: 100,
        order: Order::Desc,
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![3, 2, 1]);
}

#[test]
fn store_filters_by_kind_status_method_and_query() {
    let store = Store::new(100, "test");
    store
        .create(flow("GET", "/alpha", Some(200), Kind::Http, false))
        .expect("http flow");
    store
        .create(flow("POST", "/beta", Some(500), Kind::Sse, false))
        .expect("sse flow");
    store
        .create(flow("GET", "/gamma", None, Kind::Ws, true))
        .expect("pending flow");

    let list = |mut query: ListQuery| {
        query.limit = 100;
        store.list(&query)
    };

    let (_, rows) = list(ListQuery {
        kind: Some(Kind::Sse),
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![2]);

    let (_, rows) = list(ListQuery {
        status: StatusFilter::Error,
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![2]);

    let (_, rows) = list(ListQuery {
        status: StatusFilter::Ok,
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![1]);

    let (_, rows) = list(ListQuery {
        status: StatusFilter::Pending,
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![3]);

    let (_, rows) = list(ListQuery {
        q: Some("BETA".to_string()),
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![2], "query should be case-insensitive");

    let (_, rows) = list(ListQuery {
        method: Some("GET".to_string()),
        ..ListQuery::default()
    });
    assert_eq!(ids(&rows), vec![1, 3]);
}

#[test]
fn store_evicts_oldest_closed_flows_at_capacity() {
    let store = Store::new(2, "test");
    for path in ["/one", "/two", "/three"] {
        store
            .create(flow("GET", path, Some(200), Kind::Http, false))
            .expect("flow should be recorded");
    }

    let (total, rows) = all(&store);
    assert_eq!(total, 2);
    assert_eq!(ids(&rows), vec![2, 3]);
    assert!(store.get(1).is_none(), "oldest flow should be evicted");
    assert!(store.get(3).is_some());
}

#[test]
fn paused_store_rejects_new_flows() {
    let store = Store::new(100, "test");
    store.set_paused(true);
    assert!(store.is_paused());
    assert!(
        store
            .create(flow("GET", "/ignored", Some(200), Kind::Http, false))
            .is_none(),
        "paused stores must not record flows"
    );
    assert_eq!(all(&store).0, 0);
    assert!(store.stats().paused);

    store.set_paused(false);
    assert!(
        store
            .create(flow("GET", "/recorded", Some(200), Kind::Http, false))
            .is_some()
    );
    assert_eq!(all(&store).0, 1);
}

#[test]
fn har_export_contains_entries_and_http11_versions() {
    let store = Store::new(100, "test");
    store
        .create(flow("GET", "/item?x=1", Some(200), Kind::Http, false))
        .expect("first flow");
    let mut insecure = flow("POST", "/upload", Some(201), Kind::Http, false);
    insecure
        .notes
        .push(Note::TlsFallback("self-signed".to_string()));
    store.create(insecure).expect("second flow");

    let flows: Vec<Flow> = [1, 2]
        .into_iter()
        .map(|id| store.get(id).expect("stored flow"))
        .collect();
    let value = har::har(&flows, "9.9.9");

    assert_eq!(value["log"]["version"], "1.2");
    assert_eq!(value["log"]["creator"]["name"], "proxyhand");
    assert_eq!(value["log"]["creator"]["version"], "9.9.9");
    let entries = value["log"]["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 2);

    let first = &entries[0];
    assert_eq!(first["request"]["method"], "GET");
    assert_eq!(first["request"]["httpVersion"], "HTTP/1.1");
    assert_eq!(first["response"]["httpVersion"], "HTTP/1.1");
    assert_eq!(first["request"]["url"], "http://example.test/item?x=1");
    assert_eq!(first["request"]["queryString"][0]["name"], "x");
    assert_eq!(first["request"]["queryString"][0]["value"], "1");
    assert_eq!(first["startedDateTime"], "2023-11-14T22:13:20.000Z");
    assert!(first.get("_insecureTls").is_none());

    assert_eq!(entries[1]["_insecureTls"].as_bool(), Some(true));
    assert_eq!(entries[1]["response"]["status"], 201);

    let serialized = value.to_string();
    assert!(
        !serialized.contains("HTTP/HTTP/1.1"),
        "httpVersion must not be double-prefixed"
    );
}
