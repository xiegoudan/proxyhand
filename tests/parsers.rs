//! Parser and body-inspection unit tests for the proxyhand library.

use std::io::Write;

use flate2::Compression;
use flate2::write::GzEncoder;
use proxyhand::sse::{MAX_PARTIAL, NdjsonParser, SseFrame, SseParser};
use proxyhand::util::build_body;
use proxyhand::ws::{Direction, WsFrame, WsParser};

fn feed_sse(parser: &mut SseParser, chunk: &[u8]) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    parser.feed(chunk, &mut |frame| frames.push(frame));
    frames
}

fn feed_ndjson(parser: &mut NdjsonParser, chunk: &[u8]) -> Vec<SseFrame> {
    let mut frames = Vec::new();
    parser.feed(chunk, &mut |frame| frames.push(frame));
    frames
}

#[test]
fn sse_parses_frames_split_across_chunks() {
    let payload = concat!(
        "event: task\n",
        "id: 7\n",
        "retry: 1500\n",
        "data: first\n",
        "data: second\n",
        "\n",
        ": keep-alive\n",
        "\n",
    );
    let mut parser = SseParser::new();
    let mut frames = Vec::new();
    // Deliberately split in the middle of field names and separators.
    for chunk in payload.as_bytes().chunks(7) {
        frames.extend(feed_sse(&mut parser, chunk));
    }

    assert_eq!(frames.len(), 2, "expected two complete frames");
    let first = &frames[0];
    assert_eq!(first.i, 1);
    assert_eq!(first.event.as_deref(), Some("task"));
    assert_eq!(first.id.as_deref(), Some("7"));
    assert_eq!(first.retry, Some(1500));
    assert_eq!(first.data.as_deref(), Some("first\nsecond"));
    assert_eq!(first.comment, None);
    assert!(first.bytes > 0, "frame should record consumed bytes");
    assert!(first.ts > 0, "frame should carry an arrival timestamp");

    let second = &frames[1];
    assert_eq!(second.i, 2);
    assert_eq!(second.comment.as_deref(), Some("keep-alive"));
    assert_eq!(second.data, None);
    assert_eq!(second.event, None);
}

#[test]
fn sse_accepts_crlf_and_cr_separators() {
    let mut parser = SseParser::new();
    let frames = feed_sse(
        &mut parser,
        b"data: crlf\r\n\r\ndata: cr\r\rdata: plain\n\n",
    );

    assert_eq!(frames.len(), 3);
    assert_eq!(frames[0].data.as_deref(), Some("crlf"));
    assert_eq!(frames[1].data.as_deref(), Some("cr"));
    assert_eq!(frames[2].data.as_deref(), Some("plain"));
}

#[test]
fn sse_joins_multiline_data_and_ignores_unknown_fields() {
    let mut parser = SseParser::new();
    let frames = feed_sse(
        &mut parser,
        b"data: line1\ndata:line2\nunknown: ignored\n\n",
    );

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data.as_deref(), Some("line1\nline2"));
    assert_eq!(frames[0].event, None);
    assert_eq!(frames[0].id, None);
}

#[test]
fn sse_flushes_oversized_partial_frame() {
    // A frame without a separator is normally buffered, but the parser must not
    // grow without bound. MAX_PARTIAL is the safety valve that flushes it.
    let mut parser = SseParser::new();
    let oversized = vec![b'x'; MAX_PARTIAL + 1];
    let frames = feed_sse(&mut parser, &oversized);

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].raw.len(), MAX_PARTIAL + 1);
    assert_eq!(frames[0].bytes, 2);
    assert_eq!(frames[0].data, None);
}

#[test]
fn sse_incomplete_tail_stays_buffered() {
    let mut parser = SseParser::new();
    assert!(feed_sse(&mut parser, b"data: not done").is_empty());
    assert!(
        feed_sse(&mut parser, b" yet").is_empty(),
        "no separator means no dispatch"
    );
    let frames = feed_sse(&mut parser, b"\n\n");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data.as_deref(), Some("not done yet"));
}

#[test]
fn ndjson_parses_lines_across_chunks() {
    let mut parser = NdjsonParser::new();
    let mut frames = Vec::new();
    for chunk in b"{\"a\":1}\n\n{\"b\":2}\r\n{\"c\":3}\n".chunks(4) {
        frames.extend(feed_ndjson(&mut parser, chunk));
    }

    assert_eq!(frames.len(), 3, "blank lines should be skipped");
    assert_eq!(frames[0].data.as_deref(), Some("{\"a\":1}"));
    assert_eq!(frames[1].data.as_deref(), Some("{\"b\":2}"));
    assert_eq!(frames[2].data.as_deref(), Some("{\"c\":3}"));
    assert!(frames.iter().all(|frame| frame.comment.is_none()));
    assert_eq!(
        frames.iter().map(|frame| frame.i).collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

#[test]
fn ndjson_buffers_partial_trailing_line() {
    let mut parser = NdjsonParser::new();
    assert!(feed_ndjson(&mut parser, b"{\"d\":4}").is_empty());
    let frames = feed_ndjson(&mut parser, b"\n");
    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].data.as_deref(), Some("{\"d\":4}"));
}

fn encode_ws_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = vec![(if fin { 0x80 } else { 0 }) | opcode];
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        len @ 0..=125 => out.push(mask_bit | len as u8),
        len @ 126..=65535 => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(len as u16).to_be_bytes());
        }
        len => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(len as u64).to_be_bytes());
        }
    }
    if let Some(mask) = mask {
        out.extend_from_slice(&mask);
        out.extend(
            payload
                .iter()
                .enumerate()
                .map(|(i, byte)| byte ^ mask[i % 4]),
        );
    } else {
        out.extend_from_slice(payload);
    }
    out
}

fn feed_ws(parser: &mut WsParser, chunk: &[u8]) -> Vec<WsFrame> {
    let mut frames = Vec::new();
    parser.feed(chunk, &mut |frame| frames.push(frame));
    frames
}

#[test]
fn ws_parses_masked_client_text_frame() {
    let masked = encode_ws_frame(true, 0x1, b"hello", Some([0x37, 0xfa, 0x21, 0x3d]));
    let mut parser = WsParser::new(Direction::ClientToServer);
    assert!(feed_ws(&mut parser, &masked[..2]).is_empty());
    let frames = feed_ws(&mut parser, &masked[2..]);

    assert_eq!(frames.len(), 1);
    let frame = &frames[0];
    assert_eq!(frame.dir, Direction::ClientToServer);
    assert_eq!(frame.opcode, 0x1);
    assert_eq!(frame.opcode_name, "text");
    assert!(frame.fin);
    assert_eq!(frame.size, 5);
    assert_eq!(frame.text.as_deref(), Some("hello"));
    assert!(!frame.truncated);
}

#[test]
fn ws_parses_unmasked_server_frame_with_16_bit_length() {
    let payload = vec![b'a'; 200];
    let frame = encode_ws_frame(true, 0x1, &payload, None);
    let mut parser = WsParser::new(Direction::ServerToClient);
    let frames = feed_ws(&mut parser, &frame);

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].dir, Direction::ServerToClient);
    assert_eq!(frames[0].size, 200);
    assert_eq!(frames[0].text.as_deref(), Some("a".repeat(200).as_str()));
}

#[test]
fn ws_reassembles_fragmented_continuation() {
    let first = encode_ws_frame(false, 0x1, b"hel", Some([1, 2, 3, 4]));
    let second = encode_ws_frame(true, 0x0, b"lo", Some([5, 6, 7, 8]));
    let mut both = first;
    both.extend_from_slice(&second);

    let mut parser = WsParser::new(Direction::ClientToServer);
    let frames = feed_ws(&mut parser, &both);

    assert_eq!(frames.len(), 2);
    assert_eq!(frames[0].opcode_name, "text");
    assert_eq!(frames[0].text.as_deref(), Some("hel"));
    assert!(!frames[0].fin);
    assert_eq!(frames[1].opcode, 0x0);
    assert_eq!(frames[1].opcode_name, "continuation");
    assert!(frames[1].fin);
    assert_eq!(frames[1].text.as_deref(), Some("lo"));
}

#[test]
fn ws_renders_close_frame() {
    let mut payload = 1000u16.to_be_bytes().to_vec();
    payload.extend_from_slice(b"bye");
    let raw = encode_ws_frame(true, 0x8, &payload, None);
    let mut parser = WsParser::new(Direction::ServerToClient);
    let frames = feed_ws(&mut parser, &raw);

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].opcode, 0x8);
    assert_eq!(frames[0].opcode_name, "close");
    assert_eq!(frames[0].text.as_deref(), Some("close 1000 bye"));
}

#[test]
fn ws_renders_binary_frames_as_base64() {
    let raw = encode_ws_frame(true, 0x2, &[0x00, 0x01, 0xfe, 0xff], None);
    let mut parser = WsParser::new(Direction::ServerToClient);
    let frames = feed_ws(&mut parser, &raw);

    assert_eq!(frames.len(), 1);
    assert_eq!(frames[0].opcode_name, "binary");
    assert!(
        frames[0]
            .text
            .as_deref()
            .is_some_and(|text| text.starts_with("base64:"))
    );
}

#[test]
fn build_body_decodes_gzip() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(b"hello gzip payload").unwrap();
    let compressed = encoder.finish().unwrap();

    let view = build_body(
        std::slice::from_ref(&compressed),
        compressed.len() as u64,
        0,
        Some("text/plain; charset=utf-8"),
        Some("gzip"),
        1024,
    )
    .expect("body should be captured");

    assert_eq!(view.text.as_deref(), Some("hello gzip payload"));
    assert_eq!(view.encoding.as_deref(), Some("gzip"));
    assert_eq!(view.decoded_size, Some(18));
    assert_eq!(view.size, compressed.len() as u64);
    assert_eq!(view.mime.as_deref(), Some("text/plain"));
    assert!(!view.truncated);
    assert_eq!(view.dropped, None);
}

#[test]
fn build_body_chooses_text_or_base64() {
    let text = build_body(
        &[b"plain text".to_vec()],
        10,
        0,
        Some("text/plain"),
        None,
        1024,
    )
    .expect("text body");
    assert_eq!(text.text.as_deref(), Some("plain text"));
    assert_eq!(text.base64, None);

    let binary = build_body(
        &[vec![0x00, 0x01, 0x02, 0xff]],
        4,
        0,
        Some("application/octet-stream"),
        None,
        1024,
    )
    .expect("binary body");
    assert_eq!(binary.text, None);
    assert_eq!(binary.base64.as_deref(), Some("AAEC/w=="));
}

#[test]
fn build_body_truncates_and_reports_dropped_bytes() {
    let view = build_body(
        &[b"abcdefghij".to_vec()],
        100,
        40,
        Some("text/plain"),
        None,
        4,
    )
    .expect("truncated body");

    assert!(view.truncated);
    assert_eq!(view.dropped, Some(40));
    assert_eq!(view.size, 100);
    assert_eq!(view.text.as_deref(), Some("abcd\n… (100 bytes total)"));

    let binary = build_body(
        &[vec![0x00, 0x01, 0x02, 0xff]],
        4,
        0,
        Some("application/octet-stream"),
        None,
        2,
    )
    .expect("truncated binary body");
    assert!(binary.truncated);
    assert!(
        binary
            .base64
            .as_deref()
            .is_some_and(|value| value.ends_with('…'))
    );
}

#[test]
fn build_body_edge_cases() {
    assert!(build_body(&[], 0, 0, Some("text/plain"), None, 1024).is_none());

    let identity = build_body(
        &[b"identity".to_vec()],
        8,
        0,
        Some("text/plain"),
        Some("identity"),
        1024,
    )
    .expect("identity body");
    assert_eq!(identity.encoding, None);

    let html = build_body(
        &[b"<!doctype html><html><body>hi</body></html>".to_vec()],
        40,
        0,
        None,
        None,
        1024,
    )
    .expect("html body");
    assert_eq!(html.mime, None);
    assert!(html.text.is_some(), "HTML should be treated as textual");
}
