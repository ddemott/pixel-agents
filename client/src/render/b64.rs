//! Standard base64 (RFC 4648, padded) — shared by the image-tier encoders
//! (Kitty graphics, iTerm2 inline). No external dependency.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encode `data` to standard base64 with `=` padding.
pub fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(n >> 18) as usize & 0x3f]);
        out.push(ALPHABET[(n >> 12) as usize & 0x3f]);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 0x3f] } else { b'=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 0x3f] } else { b'=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors() {
        assert_eq!(encode(b""), b"");
        assert_eq!(encode(b"f"), b"Zg==");
        assert_eq!(encode(b"fo"), b"Zm8=");
        assert_eq!(encode(b"foo"), b"Zm9v");
        assert_eq!(encode(b"foob"), b"Zm9vYg==");
        assert_eq!(encode(b"fooba"), b"Zm9vYmE=");
        assert_eq!(encode(b"foobar"), b"Zm9vYmFy");
    }
}
