//! Incremental Server-Sent Events parser.
//!
//! Frames are flushed on a blank line (`\r\n\r\n`, `\n\n`, `\r\r`), which is what the
//! EventSource spec calls an "event dispatch". We keep the raw text of each frame so the UI can
//! show exactly what went over the wire.

use serde::Serialize;

use crate::util::now_ms;

/// Safety valve: a "line" larger than this is flushed as a frame instead of growing forever.
pub const MAX_PARTIAL: usize = 512 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SseFrame {
    pub i: u64,
    pub ts: u64,
    pub raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub bytes: u64,
}

#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    index: u64,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of the response body; `on_frame` is called for each complete event.
    pub fn feed(&mut self, chunk: &[u8], on_frame: &mut dyn FnMut(SseFrame)) {
        self.buffer.extend_from_slice(chunk);
        loop {
            let Some((end, sep_len)) = find_separator(&self.buffer) else {
                if self.buffer.len() > MAX_PARTIAL {
                    let raw = String::from_utf8_lossy(&self.buffer).into_owned();
                    self.buffer.clear();
                    self.emit(raw, 2, on_frame);
                }
                return;
            };
            let raw = String::from_utf8_lossy(&self.buffer[..end]).into_owned();
            let bytes = end as u64 + sep_len as u64;
            self.buffer.drain(..end + sep_len);
            self.emit(raw, bytes, on_frame);
        }
    }

    fn emit(&mut self, raw: String, bytes: u64, on_frame: &mut dyn FnMut(SseFrame)) {
        self.index += 1;
        let event = field(&raw, "event");
        let id = field(&raw, "id");
        let retry = field(&raw, "retry").and_then(|v| v.trim().parse().ok());
        let data = data_field(&raw);
        let comment = comments(&raw);
        on_frame(SseFrame {
            i: self.index,
            ts: now_ms(),
            raw,
            event,
            id,
            retry,
            data,
            comment,
            bytes,
        });
    }
}

/// Position and length of the first blank-line separator.
fn find_separator(buf: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == b'\r' {
            if buf.get(i + 1) == Some(&b'\n')
                && buf.get(i + 2) == Some(&b'\r')
                && buf.get(i + 3) == Some(&b'\n')
            {
                return Some((i, 4));
            }
            if buf.get(i + 1) == Some(&b'\r') {
                return Some((i, 2));
            }
        } else if buf[i] == b'\n' && buf.get(i + 1) == Some(&b'\n') {
            return Some((i, 2));
        }
        i += 1;
    }
    None
}

fn value_after<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(field)?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

fn field(raw: &str, name: &str) -> Option<String> {
    raw.split('\n')
        .filter_map(|line| value_after(line.trim_end_matches('\r'), &format!("{name}:")))
        .next()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn data_field(raw: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(v) = value_after(line, "data:") {
            lines.push(v.to_string());
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

fn comments(raw: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in raw.split('\n') {
        let line = line.trim_end_matches('\r');
        if let Some(v) = line.strip_prefix(':') {
            lines.push(v.strip_prefix(' ').unwrap_or(v).to_string());
        }
    }
    (!lines.is_empty()).then(|| lines.join("\n"))
}

/// One line of newline-delimited JSON becomes one frame.
pub struct NdjsonParser {
    buffer: Vec<u8>,
    index: u64,
}

impl Default for NdjsonParser {
    fn default() -> Self {
        Self::new()
    }
}

impl NdjsonParser {
    pub fn new() -> Self {
        Self {
            buffer: Vec::new(),
            index: 0,
        }
    }

    pub fn feed(&mut self, chunk: &[u8], on_frame: &mut dyn FnMut(SseFrame)) {
        self.buffer.extend_from_slice(chunk);
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            let raw = String::from_utf8_lossy(&self.buffer[..pos]).into_owned();
            let bytes = pos as u64 + 1;
            self.buffer.drain(..pos + 1);
            let raw = raw.trim_end_matches('\r').to_string();
            if raw.is_empty() {
                continue;
            }
            self.index += 1;
            on_frame(SseFrame {
                i: self.index,
                ts: now_ms(),
                data: Some(raw.clone()),
                raw,
                event: None,
                id: None,
                retry: None,
                comment: None,
                bytes,
            });
        }
    }
}
