//! In-memory flow store: what the proxy captures and the UI reads.
//!
//! Flows live in a map plus an insertion-order vector (chronological). Events are broadcast to
//! every connected UI client; summaries never carry notes (those are localized on demand).

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use tokio::sync::broadcast;

use crate::i18n::Lang;
use crate::sse::SseFrame;
use crate::util::{BodyView, HeaderPair, now_ms};
use crate::ws::WsFrame;

/// Upper bound on captured SSE/WS frames per flow.
pub const MAX_FRAMES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Http,
    Sse,
    Ws,
    Tunnel,
    Ndjson,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Http => "http",
            Kind::Sse => "sse",
            Kind::Ws => "ws",
            Kind::Tunnel => "tunnel",
            Kind::Ndjson => "ndjson",
        }
    }

    pub fn parse(value: &str) -> Option<Kind> {
        match value {
            "http" => Some(Kind::Http),
            "sse" => Some(Kind::Sse),
            "ws" => Some(Kind::Ws),
            "tunnel" => Some(Kind::Tunnel),
            "ndjson" => Some(Kind::Ndjson),
            _ => None,
        }
    }
}

/// Why a flow carries a warning. Kept structured so the text can follow the UI language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Note {
    TlsFallback(String),
    TunnelNotTls,
    /// The capture limit clipped this body; the flow data is incomplete.
    BodyTruncated,
}

impl Note {
    pub fn text(&self, lang: Lang) -> String {
        match self {
            Note::TlsFallback(reason) => crate::i18n::note_tls_fallback(lang, reason),
            Note::TunnelNotTls => crate::i18n::note_tunnel_not_tls(lang),
            Note::BodyTruncated => crate::i18n::note_body_truncated(lang),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CertInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowSummary {
    pub id: u64,
    pub seq: u64,
    pub scheme: String,
    pub method: String,
    pub url: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub status: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    pub kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub open: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub res_bytes: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Flow {
    pub summary: FlowSummary,
    pub http_version: String,
    pub req_headers: Vec<HeaderPair>,
    pub res_headers: Vec<HeaderPair>,
    pub req_body: Option<BodyView>,
    pub res_body: Option<BodyView>,
    pub sse_frames: Vec<SseFrame>,
    pub ws_frames: Vec<WsFrame>,
    pub ttfb_ms: Option<u64>,
    pub alpn: Option<String>,
    pub client_ip: Option<String>,
    pub cert: Option<CertInfo>,
    pub notes: Vec<Note>,
}

/// The flow as the UI sees it: summary fields flattened, notes localized.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowDto<'a> {
    #[serde(flatten)]
    pub summary: &'a FlowSummary,
    pub http_version: &'a str,
    pub req_headers: &'a [HeaderPair],
    pub res_headers: &'a [HeaderPair],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_body: Option<&'a BodyView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub res_body: Option<&'a BodyView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sse_frames: Option<&'a [SseFrame]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_frames: Option<&'a [WsFrame]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttfb_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpn: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_ip: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cert: Option<&'a CertInfo>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl Flow {
    pub fn dto(&self, lang: Lang) -> FlowDto<'_> {
        FlowDto {
            summary: &self.summary,
            http_version: &self.http_version,
            req_headers: &self.req_headers,
            res_headers: &self.res_headers,
            req_body: self.req_body.as_ref(),
            res_body: self.res_body.as_ref(),
            sse_frames: (!self.sse_frames.is_empty()).then_some(self.sse_frames.as_slice()),
            ws_frames: (!self.ws_frames.is_empty()).then_some(self.ws_frames.as_slice()),
            ttfb_ms: self.ttfb_ms,
            alpn: self.alpn.as_deref(),
            client_ip: self.client_ip.as_deref(),
            cert: self.cert.as_ref(),
            notes: self.notes.iter().map(|n| n.text(lang)).collect(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Order {
    #[default]
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StatusFilter {
    #[default]
    All,
    Ok,
    Error,
    Pending,
}

impl StatusFilter {
    pub fn parse(value: &str) -> StatusFilter {
        match value {
            "ok" => StatusFilter::Ok,
            "error" => StatusFilter::Error,
            "pending" => StatusFilter::Pending,
            _ => StatusFilter::All,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    pub limit: usize,
    pub offset: usize,
    pub order: Order,
    pub q: Option<String>,
    pub kind: Option<Kind>,
    pub method: Option<String>,
    pub status: StatusFilter,
}

/// Field-level patch; `None` means "leave as is".
#[derive(Debug, Default, Clone)]
pub struct Patch {
    pub status: Option<u16>,
    pub status_text: Option<String>,
    pub kind: Option<Kind>,
    pub content_type: Option<String>,
    pub size: Option<u64>,
    pub duration_ms: Option<u64>,
    pub ended_at: Option<u64>,
    pub error: Option<String>,
    pub open: Option<bool>,
    pub sse_count: Option<usize>,
    pub ws_count: Option<usize>,
    pub req_bytes: Option<u64>,
    pub res_bytes: Option<u64>,
    pub http_version: Option<String>,
    pub res_headers: Option<Vec<HeaderPair>>,
    pub req_headers: Option<Vec<HeaderPair>>,
    pub req_body: Option<BodyView>,
    pub res_body: Option<BodyView>,
    pub ttfb_ms: Option<u64>,
    pub alpn: Option<String>,
    pub client_ip: Option<String>,
    pub cert: Option<CertInfo>,
    pub notes: Option<Vec<Note>>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total: usize,
    pub active: usize,
    pub paused: bool,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub started_at: u64,
    pub version: String,
    pub proxy_port: u16,
    pub ui_port: u16,
    pub ca_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t")]
pub enum ServerEvent {
    #[serde(rename = "hello")]
    Hello { state: Stats },
    #[serde(rename = "flow:new")]
    New { flow: FlowSummary },
    #[serde(rename = "flow:update")]
    Update { flow: FlowSummary },
    #[serde(rename = "sse")]
    Sse { id: u64, frame: SseFrame },
    #[serde(rename = "ws")]
    Ws { id: u64, frame: WsFrame },
    #[serde(rename = "stats")]
    Stats { stats: Stats },
    #[serde(rename = "cleared")]
    Cleared,
    #[serde(rename = "paused")]
    Paused { paused: bool },
}

struct Inner {
    flows: HashMap<u64, Flow>,
    order: Vec<u64>,
    next_id: u64,
    seq: u64,
    paused: bool,
    bytes_in: u64,
    bytes_out: u64,
}

pub struct Store {
    inner: Mutex<Inner>,
    events: broadcast::Sender<ServerEvent>,
    pub max_flows: usize,
    version: String,
    started_at: u64,
    proxy_port: Mutex<u16>,
    ca_path: Mutex<String>,
}

impl Store {
    pub fn new(max_flows: usize, version: &str) -> Store {
        let (events, _) = broadcast::channel(2048);
        Store {
            inner: Mutex::new(Inner {
                flows: HashMap::new(),
                order: Vec::new(),
                next_id: 1,
                seq: 0,
                paused: false,
                bytes_in: 0,
                bytes_out: 0,
            }),
            events,
            max_flows: max_flows.max(1),
            version: version.to_string(),
            started_at: now_ms(),
            proxy_port: Mutex::new(0),
            ca_path: Mutex::new(String::new()),
        }
    }

    pub fn set_info(&self, proxy_port: u16, ca_path: &str) {
        *self.proxy_port.lock().unwrap() = proxy_port;
        *self.ca_path.lock().unwrap() = ca_path.to_string();
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ServerEvent> {
        self.events.subscribe()
    }

    fn broadcast(&self, event: ServerEvent) {
        let _ = self.events.send(event);
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn is_paused(&self) -> bool {
        self.lock().paused
    }

    pub fn add_bytes_in(&self, n: u64) {
        self.lock().bytes_in += n;
    }

    pub fn add_bytes_out(&self, n: u64) {
        self.lock().bytes_out += n;
    }

    /// Stores a freshly captured flow (unless capturing is paused) and emits `flow:new`.
    pub fn create(&self, mut flow: Flow) -> Option<Flow> {
        let stored = {
            let mut inner = self.lock();
            if inner.paused {
                return None;
            }
            inner.next_id += 1;
            inner.seq += 1;
            flow.summary.id = inner.next_id - 1;
            flow.summary.seq = inner.seq - 1;
            let id = flow.summary.id;
            inner.order.push(id);
            let stored = flow.clone();
            inner.flows.insert(id, flow);
            while inner.order.len() > self.max_flows {
                if !evict_one(&mut inner) {
                    break;
                }
            }
            stored
        };
        self.broadcast(ServerEvent::New {
            flow: stored.summary.clone(),
        });
        Some(stored)
    }

    fn get_from_inner(inner: &Inner, id: u64) -> Option<Flow> {
        inner.flows.get(&id).cloned()
    }

    pub fn get(&self, id: u64) -> Option<Flow> {
        let inner = self.lock();
        Self::get_from_inner(&inner, id)
    }

    pub fn update(&self, id: u64, patch: Patch) -> Option<FlowSummary> {
        let summary = {
            let mut inner = self.lock();
            let flow = inner.flows.get_mut(&id)?;
            apply_patch(flow, patch);
            flow.summary.clone()
        };
        self.broadcast(ServerEvent::Update {
            flow: summary.clone(),
        });
        Some(summary)
    }

    pub fn add_sse(&self, id: u64, frame: SseFrame) {
        let summary = {
            let mut inner = self.lock();
            let Some(flow) = inner.flows.get_mut(&id) else {
                return;
            };
            if flow.sse_frames.len() < MAX_FRAMES {
                flow.sse_frames.push(frame.clone());
            }
            flow.summary.sse_count = Some(flow.sse_frames.len());
            flow.summary.clone()
        };
        let _ = summary;
        self.broadcast(ServerEvent::Sse { id, frame });
    }

    pub fn add_ws(&self, id: u64, frame: WsFrame) {
        {
            let mut inner = self.lock();
            let Some(flow) = inner.flows.get_mut(&id) else {
                return;
            };
            if flow.ws_frames.len() < MAX_FRAMES {
                flow.ws_frames.push(frame.clone());
            }
            flow.summary.ws_count = Some(flow.ws_frames.len());
        }
        self.broadcast(ServerEvent::Ws { id, frame });
    }

    pub fn add_note(&self, id: u64, note: Note) {
        let mut inner = self.lock();
        if let Some(flow) = inner.flows.get_mut(&id) {
            if !flow.notes.contains(&note) {
                flow.notes.push(note);
            }
        }
    }

    pub fn list(&self, query: &ListQuery) -> (usize, Vec<FlowSummary>) {
        let inner = self.lock();
        let q = query
            .q
            .as_deref()
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty());
        let method = query
            .method
            .as_deref()
            .filter(|m| !m.eq_ignore_ascii_case("all"));
        let filtered: Vec<&Flow> = inner
            .order
            .iter()
            .filter_map(|id| inner.flows.get(id))
            .filter(|flow| {
                if let Some(kind) = query.kind {
                    if flow.summary.kind != kind {
                        return false;
                    }
                }
                if let Some(method) = method {
                    if !flow.summary.method.eq_ignore_ascii_case(method) {
                        return false;
                    }
                }
                let status = flow.summary.status;
                match query.status {
                    StatusFilter::Ok => {
                        if !matches!(status, Some(s) if s < 400) {
                            return false;
                        }
                    }
                    StatusFilter::Error => {
                        if flow.summary.error.is_none() && !matches!(status, Some(s) if s >= 400) {
                            return false;
                        }
                    }
                    StatusFilter::Pending => {
                        if !flow.summary.open {
                            return false;
                        }
                    }
                    StatusFilter::All => {}
                }
                if let Some(q) = &q {
                    let haystack = format!(
                        "{}{}{}",
                        flow.summary.host, flow.summary.path, flow.summary.method
                    )
                    .to_lowercase();
                    if !haystack.contains(q) {
                        return false;
                    }
                }
                true
            })
            .collect();

        let total = filtered.len();
        let limit = query.limit.max(1);
        let mut page: Vec<FlowSummary> = filtered
            .into_iter()
            .skip(query.offset)
            .take(limit)
            .map(|f| f.summary.clone())
            .collect();
        if query.order == Order::Desc {
            page.reverse();
        }
        (total, page)
    }

    /// Drops one flow (the API exposes this; the current UI does not use it).
    pub fn remove(&self, id: u64) -> bool {
        let mut inner = self.lock();
        if inner.flows.remove(&id).is_none() {
            return false;
        }
        inner.order.retain(|existing| *existing != id);
        true
    }

    /// Drops the flows that already finished, but keeps the ones that are still open so a live
    /// SSE or WebSocket stream the user is watching is never cut by pressing "clear".
    pub fn clear(&self) {
        {
            let mut inner = self.lock();
            let keep: Vec<u64> = inner
                .order
                .iter()
                .copied()
                .filter(|id| inner.flows.get(id).is_some_and(|flow| flow.summary.open))
                .collect();
            inner.flows.retain(|id, _| keep.contains(id));
            inner.order = keep;
            // Counters describe the flows that are still listed, not everything ever captured.
            let mut bytes_in = 0u64;
            let mut bytes_out = 0u64;
            for flow in inner.flows.values() {
                bytes_in += flow.summary.req_bytes.unwrap_or(0);
                bytes_out += flow.summary.res_bytes.unwrap_or(0);
            }
            inner.bytes_in = bytes_in;
            inner.bytes_out = bytes_out;
        }
        self.broadcast(ServerEvent::Cleared);
    }

    pub fn set_paused(&self, paused: bool) {
        self.lock().paused = paused;
        self.broadcast(ServerEvent::Paused { paused });
    }

    pub fn stats(&self) -> Stats {
        let inner = self.lock();
        // Take each guard once: a second `lock()` in the same statement would deadlock, because
        // temporaries live until the end of the enclosing statement.
        let proxy_port = *self.proxy_port.lock().unwrap_or_else(|e| e.into_inner());
        let ca_path = self
            .ca_path
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        Stats {
            total: inner.flows.len(),
            active: inner.flows.values().filter(|f| f.summary.open).count(),
            paused: inner.paused,
            bytes_in: inner.bytes_in,
            bytes_out: inner.bytes_out,
            started_at: self.started_at,
            version: self.version.clone(),
            proxy_port,
            ui_port: proxy_port,
            ca_path,
        }
    }
}

fn apply_patch(flow: &mut Flow, patch: Patch) {
    let s = &mut flow.summary;
    if let Some(v) = patch.status {
        s.status = Some(v);
        if s.status_text.is_none() {
            let text = crate::util::status_text(v);
            if !text.is_empty() {
                s.status_text = Some(text.to_string());
            }
        }
    }
    if let Some(v) = patch.status_text {
        s.status_text = Some(v);
    }
    if let Some(v) = patch.kind {
        s.kind = v;
    }
    if let Some(v) = patch.content_type {
        s.content_type = Some(v);
    }
    if let Some(v) = patch.size {
        s.size = v;
    }
    if let Some(v) = patch.duration_ms {
        s.duration_ms = Some(v);
    }
    if let Some(v) = patch.ended_at {
        s.ended_at = Some(v);
    }
    if let Some(v) = patch.error {
        s.error = Some(v);
    }
    if let Some(v) = patch.open {
        s.open = v;
        // A closing patch usually carries its own ended_at, so compute the duration from whichever
        // timestamp is authoritative instead of requiring ended_at to have been absent.
        if !v {
            let ended = s.ended_at.unwrap_or_else(now_ms);
            s.ended_at = Some(ended);
            if s.duration_ms.is_none() {
                s.duration_ms = Some(ended.saturating_sub(s.started_at));
            }
        }
    }
    if let Some(v) = patch.sse_count {
        s.sse_count = Some(v);
    }
    if let Some(v) = patch.ws_count {
        s.ws_count = Some(v);
    }
    if let Some(v) = patch.req_bytes {
        s.req_bytes = Some(v);
    }
    if let Some(v) = patch.res_bytes {
        s.res_bytes = Some(v);
    }
    if let Some(v) = patch.http_version {
        flow.http_version = v;
    }
    if let Some(v) = patch.req_headers {
        flow.req_headers = v;
    }
    if let Some(v) = patch.res_headers {
        flow.res_headers = v;
    }
    if let Some(v) = patch.req_body {
        flow.req_body = Some(v);
    }
    if let Some(v) = patch.res_body {
        flow.res_body = Some(v);
    }
    if let Some(v) = patch.ttfb_ms {
        flow.ttfb_ms = Some(v);
    }
    if let Some(v) = patch.alpn {
        flow.alpn = Some(v);
    }
    if let Some(v) = patch.client_ip {
        flow.client_ip = Some(v);
    }
    if let Some(v) = patch.cert {
        flow.cert = Some(v);
    }
    if let Some(v) = patch.notes {
        flow.notes = v;
    }
}

/// Drops the oldest closed flow. Returns false when every flow is still open (nothing evictable).
fn evict_one(inner: &mut Inner) -> bool {
    for (idx, id) in inner.order.iter().enumerate() {
        let closed = inner.flows.get(id).map(|f| !f.summary.open).unwrap_or(true);
        if closed {
            let id = *id;
            inner.order.remove(idx);
            inner.flows.remove(&id);
            return true;
        }
    }
    false
}
