//! Sixel encoder for tier T3 (arch §559/§580, 30 ms frame budget).
//!
//! Converts tightly-packed RGBA8 to a Sixel DCS string. Pixel-art sprites carry
//! few distinct colours, so the common path uses the **exact** palette (one
//! register per unique visible colour). If a sprite somehow exceeds 256 colours
//! we fall back to a 3-3-2 bit quantization (≤256 registers). Fully-transparent
//! pixels (alpha below `ALPHA_THRESHOLD`) are left undrawn — the DCS header sets
//! P2=1 so empty pixels stay at the terminal background.
//!
//! Sixel basics: the bitmap is sliced into horizontal bands 6 px tall. Within a
//! band each column is one "sixel" character (`0x3F + bits`, bit 0 = topmost
//! row). For each colour present in a band we emit `#<id>` then that colour's
//! sixels, run-length-compressed (`!<n><char>`), separated by `$` (return to the
//! band's start to overlay the next colour). `-` advances to the next band.

use std::collections::BTreeMap;

/// Alpha at or above this is opaque (drawn); below is transparent (skipped).
pub const ALPHA_THRESHOLD: u8 = 128;

/// 8-bit channel (0-255) → Sixel intensity (0-100).
fn to_sixel_channel(c: u8) -> u8 {
    ((c as u16 * 100 + 127) / 255) as u8
}

/// Per-pixel palette index (`None` = transparent) plus the colour table.
struct Quantized {
    palette: Vec<(u8, u8, u8)>, // sixel-space colours (0-100)
    indices: Vec<Option<usize>>, // len width*height
}

/// Exact palette when ≤256 unique visible colours; else 3-3-2 fallback.
fn quantize(width: u32, height: u32, rgba: &[u8]) -> Quantized {
    let n = (width * height) as usize;
    let mut map: BTreeMap<(u8, u8, u8), usize> = BTreeMap::new();
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();
    let mut indices: Vec<Option<usize>> = vec![None; n];

    for i in 0..n {
        let a = rgba[i * 4 + 3];
        if a < ALPHA_THRESHOLD {
            continue;
        }
        let c = (
            to_sixel_channel(rgba[i * 4]),
            to_sixel_channel(rgba[i * 4 + 1]),
            to_sixel_channel(rgba[i * 4 + 2]),
        );
        let idx = *map.entry(c).or_insert_with(|| {
            palette.push(c);
            palette.len() - 1
        });
        indices[i] = Some(idx);

        if palette.len() > 256 {
            return quantize_332(width, height, rgba);
        }
    }

    Quantized { palette, indices }
}

/// 3-3-2 bit fallback: ≤256 fixed buckets, compacted to the ones actually used.
fn quantize_332(width: u32, height: u32, rgba: &[u8]) -> Quantized {
    let n = (width * height) as usize;
    let mut bucket_to_idx: BTreeMap<u8, usize> = BTreeMap::new();
    let mut palette: Vec<(u8, u8, u8)> = Vec::new();
    let mut indices: Vec<Option<usize>> = vec![None; n];

    for i in 0..n {
        let a = rgba[i * 4 + 3];
        if a < ALPHA_THRESHOLD {
            continue;
        }
        let (r, g, b) = (rgba[i * 4], rgba[i * 4 + 1], rgba[i * 4 + 2]);
        let bucket = (r & 0xE0) | ((g & 0xE0) >> 3) | ((b & 0xC0) >> 6);
        let idx = *bucket_to_idx.entry(bucket).or_insert_with(|| {
            // Bucket centre as the representative colour.
            let rr = (r & 0xE0) | 0x10;
            let gg = (g & 0xE0) | 0x10;
            let bb = (b & 0xC0) | 0x20;
            palette.push((to_sixel_channel(rr), to_sixel_channel(gg), to_sixel_channel(bb)));
            palette.len() - 1
        });
        indices[i] = Some(idx);
    }

    Quantized { palette, indices }
}

/// Run-length-compress a band row of sixel chars onto `out` (`!<n><char>` for
/// runs ≥4, literals otherwise — matches common encoders and is always valid).
fn emit_rle(out: &mut Vec<u8>, ch: u8, count: usize) {
    if count >= 4 {
        out.extend_from_slice(format!("!{count}", ).as_bytes());
        out.push(ch);
    } else {
        for _ in 0..count {
            out.push(ch);
        }
    }
}

/// Encode RGBA8 (`width × height`, row-major) to a complete Sixel DCS string.
pub fn encode_sixel(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let q = quantize(width, height, rgba);
    let w = width as usize;

    let mut out = Vec::new();
    // DCS header: P1=0 (1:1 aspect), P2=1 (empty pixels transparent), P3=0.
    out.extend_from_slice(b"\x1bP0;1;0q");
    // Raster attributes: Pan;Pad;Ph;Pv = 1;1;width;height.
    out.extend_from_slice(format!("\"1;1;{width};{height}").as_bytes());

    // Colour registers: #<id>;2;<r>;<g>;<b> (2 = RGB, channels 0-100).
    for (i, &(r, g, b)) in q.palette.iter().enumerate() {
        out.extend_from_slice(format!("#{i};2;{r};{g};{b}").as_bytes());
    }

    let bands = height.div_ceil(6) as usize;
    for band in 0..bands {
        let y0 = band * 6;

        // Which palette colours appear in this band, in index order.
        let mut colors_here: Vec<usize> = Vec::new();
        {
            let mut seen = vec![false; q.palette.len()];
            for dy in 0..6 {
                let y = y0 + dy;
                if y >= height as usize {
                    break;
                }
                for x in 0..w {
                    if let Some(idx) = q.indices[y * w + x] {
                        if !seen[idx] {
                            seen[idx] = true;
                            colors_here.push(idx);
                        }
                    }
                }
            }
            colors_here.sort_unstable();
        }

        for (ci, &color) in colors_here.iter().enumerate() {
            out.extend_from_slice(format!("#{color}").as_bytes());

            // Build this colour's sixel for each column, with RLE.
            let mut run_char = 0u8;
            let mut run_len = 0usize;
            for x in 0..w {
                let mut bits = 0u8;
                for dy in 0..6 {
                    let y = y0 + dy;
                    if y >= height as usize {
                        break;
                    }
                    if q.indices[y * w + x] == Some(color) {
                        bits |= 1 << dy;
                    }
                }
                let ch = 0x3F + bits;
                if run_len == 0 {
                    run_char = ch;
                    run_len = 1;
                } else if ch == run_char {
                    run_len += 1;
                } else {
                    emit_rle(&mut out, run_char, run_len);
                    run_char = ch;
                    run_len = 1;
                }
            }
            if run_len > 0 {
                emit_rle(&mut out, run_char, run_len);
            }

            // `$` returns to band start to overlay the next colour (not after last).
            if ci + 1 < colors_here.len() {
                out.push(b'$');
            }
        }

        // `-` advances to the next band (not after the last).
        if band + 1 < bands {
            out.push(b'-');
        }
    }

    out.extend_from_slice(b"\x1b\\"); // ST
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn sixel_channel_scale() {
        assert_eq!(to_sixel_channel(0), 0);
        assert_eq!(to_sixel_channel(255), 100);
        assert_eq!(to_sixel_channel(128), 50);
    }

    #[test]
    fn single_red_pixel() {
        // 1×1 opaque red. Palette = [(100,0,0)]. Band 0, col 0: top bit set → '@'.
        let out = s(encode_sixel(1, 1, &[255, 0, 0, 255]));
        assert_eq!(out, "\x1bP0;1;0q\"1;1;1;1#0;2;100;0;0#0@\x1b\\");
    }

    #[test]
    fn fully_transparent_emits_no_pixel_data() {
        // 1×1 alpha 0 → no palette, no data, just header + raster + ST.
        let out = s(encode_sixel(1, 1, &[255, 0, 0, 0]));
        assert_eq!(out, "\x1bP0;1;0q\"1;1;1;1\x1b\\");
    }

    #[test]
    fn duplicate_color_shares_one_register() {
        // 2×1, both red → one palette entry, RLE-friendly run.
        let out = s(encode_sixel(2, 1, &[255, 0, 0, 255, 255, 0, 0, 255]));
        // One color register, then `#0` and two '@' columns (run < 4 → literals).
        assert_eq!(out, "\x1bP0;1;0q\"1;1;2;1#0;2;100;0;0#0@@\x1b\\");
        assert_eq!(out.matches("#0;2;").count(), 1, "single register");
    }

    #[test]
    fn rle_compresses_long_runs() {
        // 5×1 all red → run of 5 '@' compresses to "!5@".
        let rgba: Vec<u8> = [255, 0, 0, 255].iter().cloned().cycle().take(5 * 4).collect();
        let out = s(encode_sixel(5, 1, &rgba));
        assert!(out.contains("!5@"), "5-long run should RLE to !5@: {out}");
    }

    #[test]
    fn two_colors_overlay_with_carriage_return() {
        // 2×1: red then green. Same band, two colors → `$` between them.
        let out = s(encode_sixel(2, 1, &[255, 0, 0, 255, 0, 255, 0, 255]));
        assert!(out.contains('$'), "two colors in a band need a `$` overlay sep");
        // Two registers defined.
        assert!(out.contains("#0;2;100;0;0"));
        assert!(out.contains("#1;2;0;100;0"));
    }

    #[test]
    fn multi_band_separated_by_newline() {
        // 1×7 single color spans two bands (rows 0-5, row 6) → one `-`.
        let rgba: Vec<u8> = [0, 0, 255, 255].iter().cloned().cycle().take(7 * 4).collect();
        let out = s(encode_sixel(1, 7, &rgba));
        assert_eq!(out.matches('-').count(), 1, "7 rows = 2 bands = one newline");
    }

    #[test]
    fn second_row_sets_second_bit() {
        // 1×2 red: both rows in band 0 → bits 0b11 = 3 → char 0x3F+3 = 0x42 = 'B'.
        let out = s(encode_sixel(1, 2, &[255, 0, 0, 255, 255, 0, 0, 255]));
        assert!(out.ends_with("#0B\x1b\\"), "two stacked pixels → 'B': {out}");
    }

    #[test]
    fn header_and_terminator_present() {
        let out = s(encode_sixel(1, 1, &[1, 2, 3, 255]));
        assert!(out.starts_with("\x1bP0;1;0q\"1;1;"), "DCS header + raster attrs");
        assert!(out.ends_with("\x1b\\"), "ST terminator");
    }

    #[test]
    fn over_256_colors_falls_back_to_332() {
        // 300 distinct colors → exact path overflows, 3-3-2 fallback caps ≤256.
        let mut rgba = Vec::new();
        for i in 0..300u32 {
            rgba.extend_from_slice(&[(i & 0xff) as u8, ((i >> 1) & 0xff) as u8, ((i >> 2) & 0xff) as u8, 255]);
        }
        let q = quantize(300, 1, &rgba);
        assert!(q.palette.len() <= 256, "fallback must cap palette at 256");
        assert!(!q.palette.is_empty());
    }
}
