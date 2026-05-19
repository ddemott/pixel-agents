#![allow(dead_code)]
// Streaming frame decoder + encoder mirroring daemon/src/rpc/framing.ts §10.
//
// Wire format:
//   0x00 NDJSON : [tag][json bytes][0x0a]                              (256 KB max)
//   0x01 PTY out: [tag][streamId:u32be][len:u32be][bytes]              (1 MB max)
//   0x02 asset  : [tag][assetId:u32be][tier:u8][len:u32be][bytes]      (1 MB max)
//   0x03 PTY in : [tag][streamId:u32be][len:u32be][bytes]              (1 MB max)
//
// tier byte for asset: high bit = is_final, low 7 bits = tier number.

use bytes::{BufMut, Bytes, BytesMut};
use serde::Serialize;

pub const TAG_NDJSON: u8 = 0x00;
pub const TAG_PTY_OUT: u8 = 0x01;
pub const TAG_ASSET: u8 = 0x02;
pub const TAG_PTY_IN: u8 = 0x03;

pub const NDJSON_MAX: usize = 256 * 1024;
pub const BINARY_MAX: usize = 1024 * 1024;

// ── Error ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct FramingError(pub String);

impl std::fmt::Display for FramingError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "framing error: {}", self.0)
    }
}

impl std::error::Error for FramingError {}

// ── Frame types ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum Frame {
    Ndjson(String),
    PtyOut { stream_id: u32, bytes: Bytes },
    Asset { asset_id: u32, tier: u8, is_final: bool, bytes: Bytes },
    PtyIn { stream_id: u32, bytes: Bytes },
}

// ── Decoder ───────────────────────────────────────────────────────────────────

/// Stateful streaming decoder. Feed chunks with `push`; drain parsed frames
/// with `drain`. Once `drain` returns `Err`, the buffer is poisoned — discard
/// the decoder and close the socket; do not retry.
pub struct FrameDecoder {
    buf: BytesMut,
    poisoned: bool,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self { buf: BytesMut::new(), poisoned: false }
    }

    pub fn push(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn has_pending(&self) -> bool {
        !self.buf.is_empty()
    }

    /// Drain all complete frames from the buffer.
    /// Returns `Err` on protocol violation (oversized frame, unknown tag, bad
    /// UTF-8). After an error the decoder is poisoned and every subsequent call
    /// also returns `Err`.
    pub fn drain(&mut self) -> Result<Vec<Frame>, FramingError> {
        if self.poisoned {
            return Err(FramingError("buffer poisoned from prior protocol error".into()));
        }
        let mut frames = Vec::new();
        loop {
            match self.try_parse_one() {
                Ok(Some(f)) => frames.push(f),
                Ok(None) => return Ok(frames),
                Err(e) => {
                    self.poisoned = true;
                    return Err(e);
                }
            }
        }
    }

    fn try_parse_one(&mut self) -> Result<Option<Frame>, FramingError> {
        if self.buf.is_empty() {
            return Ok(None);
        }

        match self.buf[0] {
            TAG_NDJSON => {
                // Search for 0x0a starting at offset 1 (skip tag byte).
                let newline = self.buf[1..].iter().position(|&b| b == b'\n');

                // Guard: unterminated line that already exceeds the cap.
                if newline.is_none() && self.buf.len() - 1 > NDJSON_MAX {
                    return Err(FramingError(format!(
                        "NDJSON line exceeded {NDJSON_MAX} bytes without newline"
                    )));
                }

                let nl = match newline {
                    None => return Ok(None), // incomplete — wait for more bytes
                    // position() is relative to buf[1..], convert to absolute.
                    Some(p) => p + 1,
                };

                // nl is the absolute index of '\n' in self.buf.
                // Payload is buf[1..nl] (excludes tag and newline).
                let line_len = nl - 1;
                if line_len > NDJSON_MAX {
                    return Err(FramingError(format!(
                        "NDJSON line {line_len} bytes > {NDJSON_MAX} cap"
                    )));
                }

                // Consume tag(1) + json(line_len) + newline(1).
                let frame_bytes = self.buf.split_to(nl + 1);
                let json = std::str::from_utf8(&frame_bytes[1..nl])
                    .map_err(|e| FramingError(format!("NDJSON invalid UTF-8: {e}")))?
                    .to_owned();

                Ok(Some(Frame::Ndjson(json)))
            }

            TAG_PTY_OUT | TAG_PTY_IN => {
                // Header: tag(1) + streamId(4) + len(4) = 9 bytes.
                if self.buf.len() < 9 {
                    return Ok(None);
                }
                let stream_id = u32::from_be_bytes(self.buf[1..5].try_into().unwrap());
                let len = u32::from_be_bytes(self.buf[5..9].try_into().unwrap()) as usize;
                if len > BINARY_MAX {
                    return Err(FramingError(format!(
                        "PTY frame len {len} > {BINARY_MAX} cap (tag=0x{:02x})",
                        self.buf[0]
                    )));
                }
                if self.buf.len() < 9 + len {
                    return Ok(None);
                }

                let tag = self.buf[0];
                let _ = self.buf.split_to(9); // discard header
                let bytes = self.buf.split_to(len).freeze();

                if tag == TAG_PTY_OUT {
                    Ok(Some(Frame::PtyOut { stream_id, bytes }))
                } else {
                    Ok(Some(Frame::PtyIn { stream_id, bytes }))
                }
            }

            TAG_ASSET => {
                // Header: tag(1) + assetId(4) + tier(1) + len(4) = 10 bytes.
                if self.buf.len() < 10 {
                    return Ok(None);
                }
                let asset_id = u32::from_be_bytes(self.buf[1..5].try_into().unwrap());
                let tier_byte = self.buf[5];
                let len = u32::from_be_bytes(self.buf[6..10].try_into().unwrap()) as usize;
                if len > BINARY_MAX {
                    return Err(FramingError(format!(
                        "asset frame len {len} > {BINARY_MAX} cap"
                    )));
                }
                if self.buf.len() < 10 + len {
                    return Ok(None);
                }

                let is_final = (tier_byte & 0x80) != 0;
                let tier = tier_byte & 0x7f;
                let _ = self.buf.split_to(10); // discard header
                let bytes = self.buf.split_to(len).freeze();

                Ok(Some(Frame::Asset { asset_id, tier, is_final, bytes }))
            }

            tag => Err(FramingError(format!("unknown frame tag 0x{tag:02x}"))),
        }
    }
}

// ── Encoders ──────────────────────────────────────────────────────────────────

/// Encode a value as an outbound NDJSON frame: `[0x00][json bytes][0x0a]`.
pub fn encode_ndjson<T: Serialize>(obj: &T) -> Result<Bytes, FramingError> {
    let json =
        serde_json::to_vec(obj).map_err(|e| FramingError(format!("JSON encode error: {e}")))?;
    if json.len() > NDJSON_MAX {
        return Err(FramingError(format!("NDJSON payload {} > {NDJSON_MAX} cap", json.len())));
    }
    let mut buf = BytesMut::with_capacity(2 + json.len());
    buf.put_u8(TAG_NDJSON);
    buf.extend_from_slice(&json);
    buf.put_u8(b'\n');
    Ok(buf.freeze())
}

/// Encode a PTY-inbound frame (client → daemon keyboard input).
pub fn encode_pty_in(stream_id: u32, data: &[u8]) -> Result<Bytes, FramingError> {
    if data.len() > BINARY_MAX {
        return Err(FramingError(format!("PTY payload {} > {BINARY_MAX} cap", data.len())));
    }
    let mut buf = BytesMut::with_capacity(9 + data.len());
    buf.put_u8(TAG_PTY_IN);
    buf.put_u32(stream_id);
    buf.put_u32(data.len() as u32);
    buf.extend_from_slice(data);
    Ok(buf.freeze())
}

// Test-only encoders for roundtrip assertions.
#[cfg(test)]
pub fn encode_pty_out(stream_id: u32, data: &[u8]) -> Bytes {
    let mut buf = BytesMut::with_capacity(9 + data.len());
    buf.put_u8(TAG_PTY_OUT);
    buf.put_u32(stream_id);
    buf.put_u32(data.len() as u32);
    buf.extend_from_slice(data);
    buf.freeze()
}

#[cfg(test)]
pub fn encode_asset(asset_id: u32, tier: u8, data: &[u8], is_final: bool) -> Bytes {
    let tier_byte = if is_final { tier | 0x80 } else { tier };
    let mut buf = BytesMut::with_capacity(10 + data.len());
    buf.put_u8(TAG_ASSET);
    buf.put_u32(asset_id);
    buf.put_u8(tier_byte);
    buf.put_u32(data.len() as u32);
    buf.extend_from_slice(data);
    buf.freeze()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn decode_all(encoded: &[u8]) -> Vec<Frame> {
        let mut dec = FrameDecoder::new();
        dec.push(encoded);
        dec.drain().expect("decode failed")
    }

    // Feed bytes one at a time; assert Ok(empty) for all but last, then one frame.
    fn decode_byte_by_byte(encoded: &[u8]) -> Frame {
        let mut dec = FrameDecoder::new();
        for (i, &byte) in encoded.iter().enumerate() {
            dec.push(std::slice::from_ref(&byte));
            let frames = dec.drain().expect("decode error mid-feed");
            if i < encoded.len() - 1 {
                assert!(
                    frames.is_empty(),
                    "expected no frame after byte {i}, got {}",
                    frames.len()
                );
            } else {
                assert_eq!(frames.len(), 1, "expected exactly 1 frame on final byte");
                return frames.into_iter().next().unwrap();
            }
        }
        panic!("empty input");
    }

    // ── NDJSON ────────────────────────────────────────────────────────────────

    #[test]
    fn ndjson_roundtrip() {
        let obj = json!({ "kind": "hello", "token": "abc123" });
        let encoded = encode_ndjson(&obj).unwrap();
        let frames = decode_all(&encoded);
        assert_eq!(frames.len(), 1);
        let Frame::Ndjson(s) = &frames[0] else { panic!("not ndjson") };
        let decoded: serde_json::Value = serde_json::from_str(s).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn ndjson_byte_by_byte() {
        let obj = json!({ "method": "agent.list" });
        let encoded = encode_ndjson(&obj).unwrap();
        let frame = decode_byte_by_byte(&encoded);
        let Frame::Ndjson(s) = frame else { panic!("not ndjson") };
        let decoded: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(decoded, obj);
    }

    #[test]
    fn ndjson_multiple_frames() {
        let a = encode_ndjson(&json!({ "kind": "req", "reqId": 1 })).unwrap();
        let b = encode_ndjson(&json!({ "kind": "req", "reqId": 2 })).unwrap();
        let mut combined = BytesMut::new();
        combined.extend_from_slice(&a);
        combined.extend_from_slice(&b);
        let frames = decode_all(&combined);
        assert_eq!(frames.len(), 2);
        for (i, f) in frames.iter().enumerate() {
            let Frame::Ndjson(s) = f else { panic!("not ndjson") };
            let v: serde_json::Value = serde_json::from_str(s).unwrap();
            assert_eq!(v["reqId"], i as u64 + 1);
        }
    }

    #[test]
    fn ndjson_split_mid_payload() {
        let encoded = encode_ndjson(&json!({ "x": "hello world" })).unwrap();
        let mid = encoded.len() / 2;
        let mut dec = FrameDecoder::new();
        dec.push(&encoded[..mid]);
        assert!(dec.drain().unwrap().is_empty());
        dec.push(&encoded[mid..]);
        let frames = dec.drain().unwrap();
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn ndjson_empty_payload() {
        // Minimal valid: [0x00][{}][0x0a]
        let encoded = encode_ndjson(&json!({})).unwrap();
        let frames = decode_all(&encoded);
        assert_eq!(frames.len(), 1);
    }

    // ── PTY out ───────────────────────────────────────────────────────────────

    #[test]
    fn pty_out_roundtrip() {
        let data = b"hello from claude\r\n";
        let encoded = encode_pty_out(42, data);
        let frames = decode_all(&encoded);
        assert_eq!(frames.len(), 1);
        let Frame::PtyOut { stream_id, bytes } = &frames[0] else { panic!("not pty_out") };
        assert_eq!(*stream_id, 42);
        assert_eq!(&bytes[..], data);
    }

    #[test]
    fn pty_out_byte_by_byte() {
        let data = b"\x1b[32mGreen\x1b[0m";
        let encoded = encode_pty_out(7, data);
        let frame = decode_byte_by_byte(&encoded);
        let Frame::PtyOut { stream_id, bytes } = frame else { panic!("not pty_out") };
        assert_eq!(stream_id, 7);
        assert_eq!(&bytes[..], data);
    }

    #[test]
    fn pty_out_empty_payload() {
        let encoded = encode_pty_out(0, b"");
        let frames = decode_all(&encoded);
        assert_eq!(frames.len(), 1);
        let Frame::PtyOut { stream_id, bytes } = &frames[0] else { panic!() };
        assert_eq!(*stream_id, 0);
        assert!(bytes.is_empty());
    }

    // ── PTY in ────────────────────────────────────────────────────────────────

    #[test]
    fn pty_in_roundtrip() {
        let data = b"ls -la\r";
        let encoded = encode_pty_in(3, data).unwrap();
        let frames = decode_all(&encoded);
        assert_eq!(frames.len(), 1);
        let Frame::PtyIn { stream_id, bytes } = &frames[0] else { panic!("not pty_in") };
        assert_eq!(*stream_id, 3);
        assert_eq!(&bytes[..], data);
    }

    #[test]
    fn pty_in_byte_by_byte() {
        let data = b"cargo build\r";
        let encoded = encode_pty_in(99, data).unwrap();
        let frame = decode_byte_by_byte(&encoded);
        let Frame::PtyIn { stream_id, bytes } = frame else { panic!("not pty_in") };
        assert_eq!(stream_id, 99);
        assert_eq!(&bytes[..], data);
    }

    // ── Asset ─────────────────────────────────────────────────────────────────

    #[test]
    fn asset_roundtrip_not_final() {
        let data = b"\x89PNG\r\nchunk1";
        let encoded = encode_asset(17, 3, data, false);
        let frames = decode_all(&encoded);
        assert_eq!(frames.len(), 1);
        let Frame::Asset { asset_id, tier, is_final, bytes } = &frames[0] else {
            panic!("not asset")
        };
        assert_eq!(*asset_id, 17);
        assert_eq!(*tier, 3);
        assert!(!is_final);
        assert_eq!(&bytes[..], data);
    }

    #[test]
    fn asset_roundtrip_final() {
        let data = b"last chunk";
        let encoded = encode_asset(5, 1, data, true);
        let frames = decode_all(&encoded);
        let Frame::Asset { tier, is_final, .. } = &frames[0] else { panic!() };
        assert_eq!(*tier, 1);
        assert!(is_final);
    }

    #[test]
    fn asset_tier_max_value() {
        // tier = 0x7f (max 7-bit value); is_final = true → tier_byte = 0xff
        let encoded = encode_asset(0, 0x7f, b"x", true);
        let frames = decode_all(&encoded);
        let Frame::Asset { tier, is_final, .. } = &frames[0] else { panic!() };
        assert_eq!(*tier, 0x7f);
        assert!(is_final);
    }

    #[test]
    fn asset_byte_by_byte() {
        let data = b"sprite_data";
        let encoded = encode_asset(100, 2, data, true);
        let frame = decode_byte_by_byte(&encoded);
        let Frame::Asset { asset_id, tier, is_final, bytes } = frame else { panic!() };
        assert_eq!(asset_id, 100);
        assert_eq!(tier, 2);
        assert!(is_final);
        assert_eq!(&bytes[..], data);
    }

    // ── Mixed frames ──────────────────────────────────────────────────────────

    #[test]
    fn mixed_frame_types_sequential() {
        let ndjson = encode_ndjson(&json!({ "kind": "evt" })).unwrap();
        let pty = encode_pty_out(1, b"output");
        let asset = encode_asset(2, 0, b"blob", true);

        let mut combined = BytesMut::new();
        combined.extend_from_slice(&ndjson);
        combined.extend_from_slice(&pty);
        combined.extend_from_slice(&asset);

        let frames = decode_all(&combined);
        assert_eq!(frames.len(), 3);
        assert!(matches!(frames[0], Frame::Ndjson(_)));
        assert!(matches!(frames[1], Frame::PtyOut { .. }));
        assert!(matches!(frames[2], Frame::Asset { .. }));
    }

    // ── Error cases ───────────────────────────────────────────────────────────

    #[test]
    fn ndjson_too_large_no_newline() {
        // Build a NDJSON frame where payload > NDJSON_MAX without a newline.
        let mut buf = BytesMut::new();
        buf.put_u8(TAG_NDJSON);
        buf.extend(std::iter::repeat(b'x').take(NDJSON_MAX + 1));
        // No newline — triggers the "exceeded cap without newline" guard.
        let mut dec = FrameDecoder::new();
        dec.push(&buf);
        assert!(dec.drain().is_err());
        // Subsequent drain also errors (poisoned).
        assert!(dec.drain().is_err());
    }

    #[test]
    fn binary_too_large() {
        // PTY out with len > BINARY_MAX in the header (no actual payload needed).
        let mut buf = BytesMut::new();
        buf.put_u8(TAG_PTY_OUT);
        buf.put_u32(1); // stream_id
        buf.put_u32((BINARY_MAX + 1) as u32); // oversized len
        let mut dec = FrameDecoder::new();
        dec.push(&buf);
        assert!(dec.drain().is_err());
    }

    #[test]
    fn unknown_tag() {
        let mut dec = FrameDecoder::new();
        dec.push(&[0xFF]);
        assert!(dec.drain().is_err());
    }

    #[test]
    fn poisoned_after_error() {
        let mut dec = FrameDecoder::new();
        dec.push(&[0xFF]);
        let _ = dec.drain(); // first call: error + poison
        assert!(dec.drain().is_err()); // second call: still error
    }

    #[test]
    fn partial_pty_header_then_complete() {
        let encoded = encode_pty_out(5, b"data");
        let mut dec = FrameDecoder::new();
        // Feed only the first 4 bytes (less than 9-byte header).
        dec.push(&encoded[..4]);
        assert!(dec.drain().unwrap().is_empty());
        // Feed the rest.
        dec.push(&encoded[4..]);
        let frames = dec.drain().unwrap();
        assert_eq!(frames.len(), 1);
    }

    #[test]
    fn partial_pty_payload_then_complete() {
        let data = b"big output here";
        let encoded = encode_pty_out(1, data);
        let split = 10; // somewhere mid-payload
        let mut dec = FrameDecoder::new();
        dec.push(&encoded[..split]);
        assert!(dec.drain().unwrap().is_empty());
        dec.push(&encoded[split..]);
        let frames = dec.drain().unwrap();
        assert_eq!(frames.len(), 1);
        let Frame::PtyOut { bytes, .. } = &frames[0] else { panic!() };
        assert_eq!(&bytes[..], data);
    }
}
