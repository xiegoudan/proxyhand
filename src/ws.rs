//! Passive RFC 6455 frame observer.
//!
//! The proxy forwards WebSocket bytes untouched; this parser only rebuilds frames so the UI can
//! show them. Each frame is reported with its direction, opcode and (for text frames) contents,
//! and continuation frames are reported as they arrive (the UI shows the `fin` flag).

use serde::Serialize;

use crate::util::now_ms;

/// Give up on absurd frame sizes rather than buffering them.
pub const MAX_BUFFER: usize = 32 * 1024 * 1024;
/// Longest payload we keep verbatim in `text`.
pub const MAX_TEXT: usize = 256 * 1024;
/// Binary payloads are previewed as base64, capped here.
pub const MAX_BINARY_PREVIEW: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Direction {
    #[serde(rename = "c2s")]
    ClientToServer,
    #[serde(rename = "s2c")]
    ServerToClient,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WsFrame {
    pub i: u64,
    pub ts: u64,
    pub dir: Direction,
    pub opcode: u8,
    pub opcode_name: String,
    pub fin: bool,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

pub fn opcode_name(opcode: u8) -> &'static str {
    match opcode {
        0x0 => "continuation",
        0x1 => "text",
        0x2 => "binary",
        0x8 => "close",
        0x9 => "ping",
        0xa => "pong",
        _ => "unknown",
    }
}

pub struct WsParser {
    dir: Direction,
    buffer: Vec<u8>,
    index: u64,
    /// Whether the message being assembled started as a text frame.
    message_is_text: bool,
}

impl WsParser {
    pub fn new(dir: Direction) -> Self {
        Self {
            dir,
            buffer: Vec::new(),
            index: 0,
            message_is_text: false,
        }
    }

    pub fn feed(&mut self, chunk: &[u8], on_frame: &mut dyn FnMut(WsFrame)) {
        self.buffer.extend_from_slice(chunk);
        if self.buffer.len() > MAX_BUFFER {
            // Something is not what we think it is; drop what we have.
            self.buffer.clear();
            return;
        }
        loop {
            let Some(frame) = self.parse_one(on_frame) else {
                return;
            };
            self.emit(frame, on_frame);
        }
    }

    /// Strip one complete frame off the front of the buffer.
    fn parse_one(&mut self, _on_frame: &mut dyn FnMut(WsFrame)) -> Option<RawFrame> {
        let buf = &self.buffer;
        if buf.len() < 2 {
            return None;
        }
        let b0 = buf[0];
        let b1 = buf[1];
        let fin = b0 & 0x80 != 0;
        let opcode = b0 & 0x0f;
        let masked = b1 & 0x80 != 0;
        let len7 = (b1 & 0x7f) as u64;

        let mut header_len = 2usize;
        let payload_len = match len7 {
            126 => {
                header_len += 2;
                if buf.len() < header_len {
                    return None;
                }
                u16::from_be_bytes([buf[2], buf[3]]) as u64
            }
            127 => {
                header_len += 8;
                if buf.len() < header_len {
                    return None;
                }
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&buf[2..10]);
                u64::from_be_bytes(bytes)
            }
            n => n,
        };

        let mask_len = if masked { 4 } else { 0 };
        header_len += mask_len;
        if buf.len() < header_len {
            return None;
        }
        let mask: Option<[u8; 4]> = if masked {
            let start = header_len - 4;
            Some([buf[start], buf[start + 1], buf[start + 2], buf[start + 3]])
        } else {
            None
        };

        let total = header_len as u64 + payload_len;
        if (self.buffer.len() as u64) < total {
            return None;
        }
        let mut payload = self.buffer[header_len..total as usize].to_vec();
        self.buffer.drain(..total as usize);

        if let Some(mask) = mask {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[i % 4];
            }
        }

        if opcode == 0x1 {
            self.message_is_text = true;
        } else if opcode == 0x2 {
            self.message_is_text = false;
        }

        let mut text = None;
        let mut truncated = false;
        match opcode {
            0x1 | 0x0 if self.message_is_text => {
                let slice = &payload[..payload.len().min(MAX_TEXT)];
                let mut s = String::from_utf8_lossy(slice).into_owned();
                if payload.len() > MAX_TEXT {
                    truncated = true;
                    s.push_str(&format!(
                        "\n… (+{} bytes dropped)",
                        payload.len() - MAX_TEXT
                    ));
                }
                text = Some(s);
            }
            0x2 | 0x0 => {
                use base64::Engine as _;
                let slice = &payload[..payload.len().min(MAX_BINARY_PREVIEW)];
                let mut s = format!(
                    "base64:{}",
                    base64::engine::general_purpose::STANDARD.encode(slice)
                );
                if payload.len() > MAX_BINARY_PREVIEW {
                    truncated = true;
                    s.push_str(&format!(
                        "… (+{} bytes dropped)",
                        payload.len() - MAX_BINARY_PREVIEW
                    ));
                }
                text = Some(s);
            }
            0x8 => {
                let code = if payload.len() >= 2 {
                    u16::from_be_bytes([payload[0], payload[1]])
                } else {
                    0
                };
                let reason = String::from_utf8_lossy(&payload[2.min(payload.len())..]).into_owned();
                text = Some(format!("close {} {}", code, reason).trim_end().to_string());
            }
            _ => {}
        }

        if opcode == 0x8 || fin {
            self.message_is_text = false;
        }

        Some(RawFrame {
            opcode,
            fin,
            size: payload_len,
            text,
            truncated,
        })
    }

    fn emit(&mut self, frame: RawFrame, on_frame: &mut dyn FnMut(WsFrame)) {
        self.index += 1;
        on_frame(WsFrame {
            i: self.index,
            ts: now_ms(),
            dir: self.dir,
            opcode: frame.opcode,
            opcode_name: opcode_name(frame.opcode).to_string(),
            fin: frame.fin,
            size: frame.size,
            text: frame.text,
            truncated: frame.truncated,
        });
    }
}

struct RawFrame {
    opcode: u8,
    fin: bool,
    size: u64,
    text: Option<String>,
    truncated: bool,
}
