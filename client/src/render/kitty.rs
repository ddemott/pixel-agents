//! Kitty graphics protocol encoder for tier T1-K (unicode placeholders).
//!
//! Three pure builders, each returning bytes/text the renderer emits:
//!
//! 1. [`encode_transmit`] — upload RGBA pixels once per sprite per session
//!    (`a=t,f=32`), chunked at the protocol's 4096-base64-char limit.
//! 2. [`encode_virtual_placement`] — create a virtual placement (`a=p,U=1`)
//!    that unicode placeholders then reference, with `X=`/`Y=` sub-cell pixel
//!    offsets for pixel-exact positioning (arch §580).
//! 3. [`placeholder_text`] — the grid of `U+10EEEE` cells (row/col diacritics +
//!    the image id in the foreground colour) that actually displays the image.
//!
//! Image id = `djb2(assetId)`, matching the bytes the daemon tags asset frames
//! with. (The daemon also has an unused SHA1 `kittyIds` allocator — a daemon-side
//! inconsistency to reconcile later; the wire truth is djb2.)

use std::collections::HashSet;

use crate::assets::AssetStore;

/// The Unicode "image placeholder" character (Kitty protocol).
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// Max base64 payload bytes per transmit chunk (Kitty protocol limit).
const CHUNK_BASE64_LEN: usize = 4096;

/// Row/column diacritics (Unicode 6.0 Mn;230 combining marks), index → value.
/// Verbatim from kitty's `gen/rowcolumn-diacritics.txt`. The Nth entry encodes
/// the value N for a row or column coordinate.
pub const DIACRITICS: [u32; 297] = [
    0x0305, 0x030D, 0x030E, 0x0310, 0x0312, 0x033D, 0x033E, 0x033F, 0x0346, 0x034A, 0x034B, 0x034C,
    0x0350, 0x0351, 0x0352, 0x0357, 0x035B, 0x0363, 0x0364, 0x0365, 0x0366, 0x0367, 0x0368, 0x0369,
    0x036A, 0x036B, 0x036C, 0x036D, 0x036E, 0x036F, 0x0483, 0x0484, 0x0485, 0x0486, 0x0487, 0x0592,
    0x0593, 0x0594, 0x0595, 0x0597, 0x0598, 0x0599, 0x059C, 0x059D, 0x059E, 0x059F, 0x05A0, 0x05A1,
    0x05A8, 0x05A9, 0x05AB, 0x05AC, 0x05AF, 0x05C4, 0x0610, 0x0611, 0x0612, 0x0613, 0x0614, 0x0615,
    0x0616, 0x0617, 0x0657, 0x0658, 0x0659, 0x065A, 0x065B, 0x065D, 0x065E, 0x06D6, 0x06D7, 0x06D8,
    0x06D9, 0x06DA, 0x06DB, 0x06DC, 0x06DF, 0x06E0, 0x06E1, 0x06E2, 0x06E4, 0x06E7, 0x06E8, 0x06EB,
    0x06EC, 0x0730, 0x0732, 0x0733, 0x0735, 0x0736, 0x073A, 0x073D, 0x073F, 0x0740, 0x0741, 0x0743,
    0x0745, 0x0747, 0x0749, 0x074A, 0x07EB, 0x07EC, 0x07ED, 0x07EE, 0x07EF, 0x07F0, 0x07F1, 0x07F3,
    0x0816, 0x0817, 0x0818, 0x0819, 0x081B, 0x081C, 0x081D, 0x081E, 0x081F, 0x0820, 0x0821, 0x0822,
    0x0823, 0x0825, 0x0826, 0x0827, 0x0829, 0x082A, 0x082B, 0x082C, 0x082D, 0x0951, 0x0953, 0x0954,
    0x0F82, 0x0F83, 0x0F86, 0x0F87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
    0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71,
    0x1B72, 0x1B73, 0x1CD0, 0x1CD1, 0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4,
    0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1, 0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5,
    0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
    0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7,
    0x20DB, 0x20DC, 0x20E1, 0x20E7, 0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2,
    0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA, 0x2DEB, 0x2DEC, 0x2DED, 0x2DEE,
    0x2DEF, 0x2DF0, 0x2DF1, 0x2DF2, 0x2DF3, 0x2DF4, 0x2DF5, 0x2DF6, 0x2DF7, 0x2DF8, 0x2DF9, 0x2DFA,
    0x2DFB, 0x2DFC, 0x2DFD, 0x2DFE, 0x2DFF, 0xA66F, 0xA67C, 0xA67D, 0xA6F0, 0xA6F1, 0xA8E0, 0xA8E1,
    0xA8E2, 0xA8E3, 0xA8E4, 0xA8E5, 0xA8E6, 0xA8E7, 0xA8E8, 0xA8E9, 0xA8EA, 0xA8EB, 0xA8EC, 0xA8ED,
    0xA8EE, 0xA8EF, 0xA8F0, 0xA8F1, 0xAAB0, 0xAAB2, 0xAAB3, 0xAAB7, 0xAAB8, 0xAABE, 0xAABF, 0xAAC1,
    0xFE20, 0xFE21, 0xFE22, 0xFE23, 0xFE24, 0xFE25, 0xFE26, 0x10A0F, 0x10A38, 0x1D185, 0x1D186, 0x1D187,
    0x1D188, 0x1D189, 0x1D1AA, 0x1D1AB, 0x1D1AC, 0x1D1AD, 0x1D242, 0x1D243, 0x1D244,
];

/// The diacritic char encoding coordinate `n` (saturating at the table end).
fn diacritic(n: usize) -> char {
    let cp = DIACRITICS[n.min(DIACRITICS.len() - 1)];
    char::from_u32(cp).expect("diacritics table holds valid scalar values")
}

// ── Base64 (standard alphabet, padded) ──────────────────────────────────────

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(B64[(n >> 18) as usize & 0x3f]);
        out.push(B64[(n >> 12) as usize & 0x3f]);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 0x3f] } else { b'=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 0x3f] } else { b'=' });
    }
    out
}

// ── 1. Transmit ─────────────────────────────────────────────────────────────

/// Build the APC transmit command(s) uploading `rgba` (32-bit, `width×height`)
/// under image id `image_id`. Pixels are uploaded at native size — Kitty scales
/// at placement time, so we never pre-scale per zoom. Control keys ride only on
/// the first chunk; continuations carry `m=1`, the final carries `m=0`.
pub fn encode_transmit(image_id: u32, width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let payload = base64_encode(rgba);
    let mut out = Vec::new();

    let chunks: Vec<&[u8]> = if payload.is_empty() {
        vec![&payload[..]]
    } else {
        payload.chunks(CHUNK_BASE64_LEN).collect()
    };
    let last = chunks.len() - 1;

    for (i, chunk) in chunks.iter().enumerate() {
        out.extend_from_slice(b"\x1b_G");
        if i == 0 {
            // First chunk carries all control keys.
            out.extend_from_slice(
                format!("a=t,f=32,s={width},v={height},i={image_id},").as_bytes(),
            );
        }
        let more = if i == last { 0 } else { 1 };
        out.extend_from_slice(format!("m={more}").as_bytes());
        out.push(b';');
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out
}

// ── 2. Virtual placement ─────────────────────────────────────────────────────

/// Build the virtual-placement command for an already-transmitted image. `cols`
/// and `rows` size the placement grid; `x_off`/`y_off` are pixel offsets into
/// the first cell for sub-cell-exact positioning. `U=1` marks it virtual so the
/// unicode placeholders from [`placeholder_text`] bind to it.
pub fn encode_virtual_placement(
    image_id: u32,
    placement_id: u32,
    cols: u16,
    rows: u16,
    x_off: u16,
    y_off: u16,
) -> Vec<u8> {
    let mut s = format!("\x1b_Ga=p,U=1,i={image_id},p={placement_id},c={cols},r={rows}");
    if x_off > 0 {
        s.push_str(&format!(",X={x_off}"));
    }
    if y_off > 0 {
        s.push_str(&format!(",Y={y_off}"));
    }
    s.push_str(";\x1b\\");
    s.into_bytes()
}

// ── 3. Placeholder text ──────────────────────────────────────────────────────

/// Build the `cols×rows` block of placeholder cells that displays `image_id`.
///
/// Each row is: an SGR setting the foreground to the image id's low 24 bits,
/// then per cell `U+10EEEE` + row-diacritic + col-diacritic + (on the first
/// cell of the row) a third diacritic carrying the image id's high byte. A
/// trailing SGR reset closes the row. Rows are newline-separated; the caller
/// positions the cursor at the top-left cell before emitting.
pub fn placeholder_text(image_id: u32, cols: u16, rows: u16) -> String {
    // Image id's low 24 bits ride in the fg colour, big-endian: R = bits 16-23,
    // G = bits 8-15, B = bits 0-7. Bits 24-31 ride in the 3rd diacritic.
    let r = ((image_id >> 16) & 0xff) as u8;
    let g = ((image_id >> 8) & 0xff) as u8;
    let b = (image_id & 0xff) as u8;
    let high = ((image_id >> 24) & 0xff) as usize;

    let mut out = String::new();
    for row in 0..rows as usize {
        out.push_str(&format!("\x1b[38;2;{r};{g};{b}m"));
        for col in 0..cols as usize {
            out.push(PLACEHOLDER);
            out.push(diacritic(row));
            out.push(diacritic(col));
            // Third diacritic (high byte of id) only needs to appear once per
            // run; emit on the first cell of each row for robustness.
            if col == 0 {
                out.push(diacritic(high));
            }
        }
        out.push_str("\x1b[0m");
        if row + 1 < rows as usize {
            out.push('\n');
        }
    }
    out
}

// ── Upload orchestration ─────────────────────────────────────────────────────

/// Uses `string_asset_id` as the Kitty image id (see module docs).
fn image_id_for(asset_id: &str) -> u32 {
    crate::assets::string_asset_id(asset_id)
}

/// Tracks which sprite image ids have been transmitted this session so each PNG
/// uploads to the terminal exactly once (image data is cached terminal-side by
/// `i=<id>`; placements/placeholders are cheap thereafter — arch §565).
#[derive(Default)]
pub struct KittyUploader {
    uploaded: HashSet<u32>,
}

impl KittyUploader {
    pub fn new() -> Self {
        Self::default()
    }

    /// Build transmit bytes for every decoded asset not yet uploaded, marking
    /// them uploaded. `a=t` is display-free, so the caller may write the result
    /// out-of-band of the Ratatui draw without disturbing the cell grid. Empty
    /// once every sprite has been sent.
    pub fn pending_uploads(&mut self, assets: &AssetStore) -> Vec<u8> {
        let mut out = Vec::new();
        for (asset_id, decoded) in assets.iter() {
            let id = image_id_for(asset_id);
            if self.uploaded.insert(id) {
                out.extend_from_slice(&encode_transmit(
                    id,
                    decoded.width,
                    decoded.height,
                    &decoded.rgba,
                ));
            }
        }
        out
    }

    pub fn uploaded_count(&self) -> usize {
        self.uploaded.len()
    }
}

// ── Placement geometry ──────────────────────────────────────────────────────

/// Where a sprite lands on the cell grid: the top-left cell, the placement size
/// in cells, and the sub-cell pixel offset into that first cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub cell_col: u16,
    pub cell_row: u16,
    pub cols: u16,
    pub rows: u16,
    pub x_off: u16,
    pub y_off: u16,
}

/// Map a sprite at device-pixel `(dev_x, dev_y)` of size `dev_w × dev_h` onto a
/// `cell_w × cell_h` cell grid. The sub-cell offset (`dev % cell`) feeds the
/// placement's `X=`/`Y=` keys; `cols`/`rows` grow to cover the offset spill so
/// the whole sprite is addressable (arch §580, pixel-exact in T1-K).
///
/// All inputs are device pixels — the caller scales world pixels by `zoom`.
pub fn compute_placement(
    cell_w: u16,
    cell_h: u16,
    dev_x: u32,
    dev_y: u32,
    dev_w: u32,
    dev_h: u32,
) -> Placement {
    let cw = cell_w.max(1) as u32;
    let ch = cell_h.max(1) as u32;
    let x_off = (dev_x % cw) as u16;
    let y_off = (dev_y % ch) as u16;
    Placement {
        cell_col: (dev_x / cw) as u16,
        cell_row: (dev_y / ch) as u16,
        cols: (((x_off as u32) + dev_w).div_ceil(cw)).max(1) as u16,
        rows: (((y_off as u32) + dev_h).div_ceil(ch)).max(1) as u16,
        x_off,
        y_off,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), b"");
        assert_eq!(base64_encode(b"f"), b"Zg==");
        assert_eq!(base64_encode(b"fo"), b"Zm8=");
        assert_eq!(base64_encode(b"foo"), b"Zm9v");
        assert_eq!(base64_encode(b"foob"), b"Zm9vYg==");
    }

    #[test]
    fn transmit_single_chunk_1x1_red() {
        // 1×1 opaque red RGBA = [255,0,0,255] → base64 "/wAA/w=="
        let bytes = encode_transmit(7, 1, 1, &[255, 0, 0, 255]);
        let s = String::from_utf8(bytes).unwrap();
        assert_eq!(s, "\x1b_Ga=t,f=32,s=1,v=1,i=7,m=0;/wAA/w==\x1b\\");
    }

    #[test]
    fn transmit_chunks_control_keys_first_only() {
        // Large payload forces >1 chunk. 4096 base64 chars = 3072 raw bytes.
        let raw = vec![0xABu8; 4096]; // → 5464 base64 chars → 2 chunks
        let bytes = encode_transmit(42, 32, 32, &raw);
        let s = String::from_utf8(bytes).unwrap();

        let frames: Vec<&str> = s.split("\x1b\\").filter(|f| !f.is_empty()).collect();
        assert_eq!(frames.len(), 2, "expected exactly two transmit chunks");

        // First chunk: control keys present, m=1 (more coming).
        assert!(frames[0].contains("a=t,f=32,s=32,v=32,i=42,"));
        assert!(frames[0].contains("m=1;"));
        // Continuation chunk: no control keys, m=0 (final).
        assert!(!frames[1].contains("a=t"));
        assert!(!frames[1].contains("i=42"));
        assert!(frames[1].contains("m=0;"));

        // Split point: first chunk's payload is exactly CHUNK_BASE64_LEN chars.
        let payload0 = frames[0].split(';').nth(1).unwrap();
        assert_eq!(payload0.len(), CHUNK_BASE64_LEN);
    }

    #[test]
    fn virtual_placement_with_offsets() {
        let s = String::from_utf8(encode_virtual_placement(7, 1, 4, 2, 3, 5)).unwrap();
        assert_eq!(s, "\x1b_Ga=p,U=1,i=7,p=1,c=4,r=2,X=3,Y=5;\x1b\\");
    }

    #[test]
    fn virtual_placement_omits_zero_offsets() {
        let s = String::from_utf8(encode_virtual_placement(7, 1, 4, 2, 0, 0)).unwrap();
        assert_eq!(s, "\x1b_Ga=p,U=1,i=7,p=1,c=4,r=2;\x1b\\");
        assert!(!s.contains("X="));
        assert!(!s.contains("Y="));
    }

    #[test]
    fn placeholder_fg_color_is_low_24_bits_big_endian() {
        // Kitty: low 24 bits of the id big-endian → R=bits16-23, G=bits8-15, B=bits0-7.
        // id 0x010203 → R=1, G=2, B=3.
        let t = placeholder_text(0x01_02_03, 2, 2);
        assert!(t.contains("\x1b[38;2;1;2;3m"), "fg should carry low 24 bits big-endian");

        // Independent vector: id 27 (0x00_00_1B) → R=0, G=0, B=27.
        let t27 = placeholder_text(27, 1, 1);
        assert!(t27.contains("\x1b[38;2;0;0;27m"), "id 27 → fg 0;0;27");
    }

    #[test]
    fn placeholder_grid_shape_and_diacritics() {
        let t = placeholder_text(0xAB, 2, 2);
        // 4 placeholder chars total.
        assert_eq!(t.matches(PLACEHOLDER).count(), 4);

        // Row 0 col 0 cell: placeholder + diac(0) + diac(0) + diac(high=0).
        let d0 = diacritic(0);
        let d1 = diacritic(1);
        // First cell uses (row0,col0) = (d0,d0); the (0,1) cell uses (d0,d1).
        let cell00: String = [PLACEHOLDER, d0, d0, diacritic(0)].iter().collect();
        assert!(t.contains(&cell00));
        let cell01: String = [PLACEHOLDER, d0, d1].iter().collect();
        assert!(t.contains(&cell01));

        // Two rows separated by one newline.
        assert_eq!(t.matches('\n').count(), 1);
        // Each row resets SGR.
        assert_eq!(t.matches("\x1b[0m").count(), 2);
    }

    #[test]
    fn placement_aligned_sprite() {
        // 32×24 sprite at zoom 2 = 64×48 device px, origin (0,0), 8×16 cells.
        let p = compute_placement(8, 16, 0, 0, 64, 48);
        assert_eq!(p, Placement { cell_col: 0, cell_row: 0, cols: 8, rows: 3, x_off: 0, y_off: 0 });
    }

    #[test]
    fn placement_sub_cell_offset_grows_grid() {
        // Origin (3,5): offsets push the sprite, needing one extra col/row.
        let p = compute_placement(8, 16, 3, 5, 16, 16);
        assert_eq!(p.cell_col, 0);
        assert_eq!(p.cell_row, 0);
        assert_eq!(p.x_off, 3);
        assert_eq!(p.y_off, 5);
        // 3+16=19 px over 8-px cells → 3 cols; 5+16=21 over 16-px → 2 rows.
        assert_eq!(p.cols, 3);
        assert_eq!(p.rows, 2);
    }

    #[test]
    fn placement_cell_index_from_device_px() {
        // dev (20,40): col 20/8=2 (rem 4), row 40/16=2 (rem 8).
        let p = compute_placement(8, 16, 20, 40, 8, 16);
        assert_eq!((p.cell_col, p.cell_row), (2, 2));
        assert_eq!((p.x_off, p.y_off), (4, 8));
    }

    #[test]
    fn uploader_dedupes_per_session() {
        // Build a store with one decoded 1×1 sprite via the public frame path.
        let png = {
            let mut buf = image::RgbaImage::new(1, 1);
            buf.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
            let mut out = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(buf)
                .write_to(&mut out, image::ImageFormat::Png)
                .unwrap();
            out.into_inner()
        };
        let mut assets = AssetStore::new();
        let nid = assets.register_request("DESK");
        assets.on_frame(nid, 0, true, &png).unwrap();

        let mut up = KittyUploader::new();
        let first = up.pending_uploads(&assets);
        assert!(!first.is_empty(), "first call uploads the sprite");
        assert_eq!(up.uploaded_count(), 1);

        let second = up.pending_uploads(&assets);
        assert!(second.is_empty(), "already-uploaded sprite is not re-sent");
        assert_eq!(up.uploaded_count(), 1);
    }

    #[test]
    fn diacritics_table_complete_and_valid() {
        assert_eq!(DIACRITICS.len(), 297);
        assert_eq!(DIACRITICS[0], 0x0305);
        assert_eq!(DIACRITICS[296], 0x1D244);
        for &cp in DIACRITICS.iter() {
            assert!(char::from_u32(cp).is_some(), "{cp:#x} is a valid scalar");
        }
    }
}
