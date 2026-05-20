//! iTerm2 inline-image encoder for tier T2 (arch §580/§559).
//!
//! iTerm2 (and compatible terminals: WezTerm, recent Konsole) display an image
//! at the cursor via the OSC 1337 `File=` sequence carrying a base64 image file.
//! Unlike Kitty there's no id cache — the bytes ride inline on every placement,
//! so T2 re-sends the PNG per draw (arch notes the quadrant-dirty mitigation for
//! the eventual Day 17 compositor).
//!
//! Sizing uses **cell** units (`width=<cols>;height=<rows>`) so a sprite lands on
//! an exact cell rectangle; `preserveAspectRatio=0` lets it fill that rectangle
//! without letterboxing. Move the cursor with [`crate::render::kitty::cursor_to`]
//! before emitting.

use crate::render::b64;

/// Build the OSC 1337 inline-image sequence for `image_bytes` (a complete image
/// file — PNG in our pipeline) sized to `cols × rows` terminal cells.
///
/// Envelope: `ESC ] 1337 ; File=inline=1;size=<n>;width=<cols>;height=<rows>;
/// preserveAspectRatio=0 : <base64> BEL`.
pub fn encode_inline(image_bytes: &[u8], cols: u16, rows: u16) -> Vec<u8> {
    let payload = b64::encode(image_bytes);
    let mut out = Vec::with_capacity(payload.len() + 64);
    out.extend_from_slice(b"\x1b]1337;File=inline=1");
    out.extend_from_slice(format!(";size={}", image_bytes.len()).as_bytes());
    out.extend_from_slice(format!(";width={cols};height={rows}").as_bytes());
    out.extend_from_slice(b";preserveAspectRatio=0:");
    out.extend_from_slice(&payload);
    out.push(0x07); // BEL terminator
    out
}

/// Re-encode tightly-packed RGBA8 (`width × height`) to PNG bytes, the file
/// format T2 ships inline. The asset pipeline decodes incoming PNGs to RGBA;
/// this reverses that so iTerm2 (and any future PNG consumer) has a file to send.
pub fn rgba_to_png(width: u32, height: u32, rgba: &[u8]) -> anyhow::Result<Vec<u8>> {
    let buf = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| anyhow::anyhow!("rgba buffer wrong size for {width}x{height}"))?;
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(buf).write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_envelope_structure() {
        // 3-byte payload "foo" → base64 "Zm9v".
        let s = String::from_utf8(encode_inline(b"foo", 8, 4)).unwrap();
        assert_eq!(
            s,
            "\x1b]1337;File=inline=1;size=3;width=8;height=4;preserveAspectRatio=0:Zm9v\x07"
        );
    }

    #[test]
    fn inline_ends_with_bel_and_carries_size() {
        let bytes = encode_inline(&[0u8; 10], 1, 1);
        assert_eq!(*bytes.last().unwrap(), 0x07, "must terminate with BEL");
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains(";size=10"), "size = raw byte length, not base64 length");
        assert!(s.starts_with("\x1b]1337;File=inline=1"));
    }

    #[test]
    fn rgba_to_png_roundtrips_dims() {
        let rgba = vec![7u8; 2 * 3 * 4]; // 2×3 RGBA
        let png = rgba_to_png(2, 3, &rgba).unwrap();
        // Decodes back to the same dimensions.
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
            .unwrap()
            .to_rgba8();
        assert_eq!((img.width(), img.height()), (2, 3));
    }

    #[test]
    fn rgba_to_png_rejects_wrong_buffer_size() {
        assert!(rgba_to_png(2, 2, &[0u8; 7]).is_err());
    }
}
