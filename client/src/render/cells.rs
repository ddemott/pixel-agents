//! Cell-tier sprite rasterizers (arch §559/§584) — tiers T4/T5/T6 + T6b.
//!
//! These paint an RGBA sprite into a Ratatui [`Buffer`] using text cells, for
//! terminals without graphics protocols:
//!
//! - [`rasterize_halfblock`] — T4 (24-bit), T5 (256), T6 (16). One *cell* packs
//!   two vertical pixels via `▀` (fg = top px, bg = bottom px). T4/T5/T6 share
//!   this exact rasterization (arch §585 "same geometry as T4 quantized"); the
//!   tier only signals colour depth and the *terminal* down-quantizes the
//!   truecolor we emit. So there's no separate 256/16 palette mapper here.
//! - [`rasterize_braille`] — T6b. One cell is a 2×4 dot grid (`U+2800 + bits`),
//!   monochrome-ish (fg = mean of lit pixels). Last-resort sub-cell precision.
//!
//! Horizontal pixel doubling (arch §591): a cell is ~2:1 (h:w), so half-block
//! maps 2 px vertically but 1 px horizontally per cell — sprites look 2× wide.
//! This is accepted, not mitigated (arch rejected per-axis zoom). Transparent
//! pixels (alpha < `ALPHA_THRESHOLD`) never overwrite the destination cell.

use ratatui::buffer::Buffer;
use ratatui::style::Color;

/// Alpha at or above this is opaque (painted); below is transparent (skipped).
pub const ALPHA_THRESHOLD: u8 = 128;

/// Sample sprite pixel `(x, y)` → `Some(Color)` if opaque, else `None`.
fn sample(rgba: &[u8], w: u32, h: u32, x: u32, y: u32) -> Option<Color> {
    if x >= w || y >= h {
        return None;
    }
    let i = ((y * w + x) * 4) as usize;
    if rgba.get(i + 3).copied().unwrap_or(0) < ALPHA_THRESHOLD {
        return None;
    }
    Some(Color::Rgb(rgba[i], rgba[i + 1], rgba[i + 2]))
}

/// True if `(x, y)` is inside the buffer's area.
fn in_bounds(buf: &Buffer, x: u16, y: u16) -> bool {
    let a = buf.area;
    x >= a.left() && x < a.right() && y >= a.top() && y < a.bottom()
}

/// Half-block rasterize (T4/T5/T6). Cell origin `(ox, oy)` is the top-left cell;
/// the sprite occupies `w` columns × `ceil(h/2)` rows. Clipped to `buf`'s area.
pub fn rasterize_halfblock(rgba: &[u8], w: u32, h: u32, buf: &mut Buffer, ox: u16, oy: u16) {
    let rows = h.div_ceil(2);
    for cy in 0..rows {
        for cx in 0..w {
            let dx = ox as u32 + cx;
            let dy = oy as u32 + cy;
            if dx > u16::MAX as u32 || dy > u16::MAX as u32 {
                continue;
            }
            let (dx, dy) = (dx as u16, dy as u16);
            if !in_bounds(buf, dx, dy) {
                continue;
            }
            let top = sample(rgba, w, h, cx, cy * 2);
            let bot = sample(rgba, w, h, cx, cy * 2 + 1);
            let cell = &mut buf[(dx, dy)];
            match (top, bot) {
                (Some(t), Some(b)) => {
                    cell.set_char('▀');
                    cell.set_fg(t);
                    cell.set_bg(b);
                }
                // Bottom transparent: upper-half block, default bg shows through.
                (Some(t), None) => {
                    cell.set_char('▀');
                    cell.set_fg(t);
                }
                // Top transparent: lower-half block.
                (None, Some(b)) => {
                    cell.set_char('▄');
                    cell.set_fg(b);
                }
                (None, None) => {} // fully transparent → leave destination untouched
            }
        }
    }
}

/// Braille dot bit per `[sub_col][sub_row]` (Unicode 2×4 layout: left col dots
/// 1,2,3,7 → bits 0,1,2,6; right col dots 4,5,6,8 → bits 3,4,5,7).
const BRAILLE_BITS: [[u8; 4]; 2] = [[0, 1, 2, 6], [3, 4, 5, 7]];

/// Braille rasterize (T6b). Each cell is a 2×4 px block; the sprite occupies
/// `ceil(w/2)` columns × `ceil(h/4)` rows. fg = mean colour of lit pixels.
pub fn rasterize_braille(rgba: &[u8], w: u32, h: u32, buf: &mut Buffer, ox: u16, oy: u16) {
    let cols = w.div_ceil(2);
    let rows = h.div_ceil(4);
    for cy in 0..rows {
        for cx in 0..cols {
            let dx = ox as u32 + cx;
            let dy = oy as u32 + cy;
            if dx > u16::MAX as u32 || dy > u16::MAX as u32 {
                continue;
            }
            let (dx, dy) = (dx as u16, dy as u16);
            if !in_bounds(buf, dx, dy) {
                continue;
            }
            let mut bits: u8 = 0;
            let (mut rs, mut gs, mut bs, mut n) = (0u32, 0u32, 0u32, 0u32);
            for (sx, col_bits) in BRAILLE_BITS.iter().enumerate() {
                for (sy, &bit) in col_bits.iter().enumerate() {
                    if let Some(Color::Rgb(r, g, b)) =
                        sample(rgba, w, h, cx * 2 + sx as u32, cy * 4 + sy as u32)
                    {
                        bits |= 1 << bit;
                        rs += r as u32;
                        gs += g as u32;
                        bs += b as u32;
                        n += 1;
                    }
                }
            }
            if bits != 0 {
                let cell = &mut buf[(dx, dy)];
                cell.set_char(char::from_u32(0x2800 + bits as u32).unwrap());
                cell.set_fg(Color::Rgb((rs / n) as u8, (gs / n) as u8, (bs / n) as u8));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    /// 2×2 RGBA: TL red, TR green, BL blue, BR white (all opaque).
    fn quad() -> Vec<u8> {
        vec![
            255, 0, 0, 255, /* TL red   */ 0, 255, 0, 255, /* TR green */
            0, 0, 255, 255, /* BL blue  */ 255, 255, 255, 255, /* BR white */
        ]
    }

    fn buf(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    #[test]
    fn halfblock_packs_two_rows_into_one_cell() {
        let mut b = buf(2, 1);
        rasterize_halfblock(&quad(), 2, 2, &mut b, 0, 0);
        // Cell (0,0): top=red, bottom=blue.
        assert_eq!(b[(0, 0)].symbol(), "▀");
        assert_eq!(b[(0, 0)].fg, Color::Rgb(255, 0, 0));
        assert_eq!(b[(0, 0)].bg, Color::Rgb(0, 0, 255));
        // Cell (1,0): top=green, bottom=white.
        assert_eq!(b[(1, 0)].fg, Color::Rgb(0, 255, 0));
        assert_eq!(b[(1, 0)].bg, Color::Rgb(255, 255, 255));
    }

    #[test]
    fn halfblock_transparent_bottom_uses_upper_block_no_bg() {
        // 1×2: top red opaque, bottom transparent.
        let rgba = vec![255, 0, 0, 255, 0, 0, 0, 0];
        let mut b = buf(1, 1);
        rasterize_halfblock(&rgba, 1, 2, &mut b, 0, 0);
        assert_eq!(b[(0, 0)].symbol(), "▀");
        assert_eq!(b[(0, 0)].fg, Color::Rgb(255, 0, 0));
        assert_eq!(b[(0, 0)].bg, Color::Reset); // untouched
    }

    #[test]
    fn halfblock_transparent_top_uses_lower_block() {
        // 1×2: top transparent, bottom blue.
        let rgba = vec![0, 0, 0, 0, 0, 0, 255, 255];
        let mut b = buf(1, 1);
        rasterize_halfblock(&rgba, 1, 2, &mut b, 0, 0);
        assert_eq!(b[(0, 0)].symbol(), "▄");
        assert_eq!(b[(0, 0)].fg, Color::Rgb(0, 0, 255));
    }

    #[test]
    fn halfblock_fully_transparent_cell_untouched() {
        let rgba = vec![0, 0, 0, 0, 0, 0, 0, 0];
        let mut b = buf(1, 1);
        rasterize_halfblock(&rgba, 1, 2, &mut b, 0, 0);
        assert_eq!(b[(0, 0)].symbol(), " "); // empty default
    }

    #[test]
    fn halfblock_clips_outside_buffer() {
        let mut b = buf(1, 1);
        // Origin past the buffer — must not panic, must not paint.
        rasterize_halfblock(&quad(), 2, 2, &mut b, 5, 5);
        assert_eq!(b[(0, 0)].symbol(), " ");
    }

    #[test]
    fn braille_sets_correct_dot_bits() {
        // 2×4: light only the two top corners → dot1 (bit0) + dot4 (bit3).
        let mut rgba = vec![0u8; 2 * 4 * 4];
        let set = |v: &mut Vec<u8>, x: usize, y: usize, c: [u8; 4]| {
            let i = (y * 2 + x) * 4;
            v[i..i + 4].copy_from_slice(&c);
        };
        set(&mut rgba, 0, 0, [255, 0, 0, 255]); // (0,0) → bit0
        set(&mut rgba, 1, 0, [255, 0, 0, 255]); // (1,0) → bit3
        let mut b = buf(1, 1);
        rasterize_braille(&rgba, 2, 4, &mut b, 0, 0);
        // bits = 0b1001 = 0x09 → U+2809.
        assert_eq!(b[(0, 0)].symbol(), "\u{2809}");
        assert_eq!(b[(0, 0)].fg, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn braille_empty_cell_untouched() {
        let rgba = vec![0u8; 2 * 4 * 4];
        let mut b = buf(1, 1);
        rasterize_braille(&rgba, 2, 4, &mut b, 0, 0);
        assert_eq!(b[(0, 0)].symbol(), " ");
    }

    #[test]
    fn braille_bottom_row_uses_dots_7_and_8() {
        // (0,3) → bit6 (dot7), (1,3) → bit7 (dot8) → bits 0b11000000 = 0xC0.
        let mut rgba = vec![0u8; 2 * 4 * 4];
        let i03 = (3 * 2) * 4;
        let i13 = (3 * 2 + 1) * 4;
        rgba[i03..i03 + 4].copy_from_slice(&[10, 20, 30, 255]);
        rgba[i13..i13 + 4].copy_from_slice(&[10, 20, 30, 255]);
        let mut b = buf(1, 1);
        rasterize_braille(&rgba, 2, 4, &mut b, 0, 0);
        assert_eq!(b[(0, 0)].symbol(), "\u{28C0}");
    }
}
