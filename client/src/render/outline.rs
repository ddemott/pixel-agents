//! White selection outline (Phase 3 Day 21).
//!
//! Port of `getOutlineSprite` (`spriteCache.ts`): given a sprite, produce a
//! 1px white outline that is 2px larger in each dimension. Each opaque source
//! pixel marks its 4 cardinal neighbours white; pixels overlapping the original
//! opaque area are then cleared so the outline is a hollow ring. Rendered just
//! behind the character (lower z) when it is the selected agent.
//!
//! Webview draws the outline at 100% alpha for selection, 50% for hover. Cell
//! tiers can't alpha-blend, so the outline is solid white; hover is not wired
//! (selection only, sourced from focus). Real alpha lands with the image tiers.

use crate::assets::DecodedAsset;

const WHITE: [u8; 4] = [255, 255, 255, 255];

/// Build a `(w+2)×(h+2)` white 1px outline of `src`'s opaque silhouette.
pub fn outline_sprite(src: &DecodedAsset) -> DecodedAsset {
    let w = src.width;
    let h = src.height;
    let ow = w + 2;
    let oh = h + 2;
    let mut out = vec![0u8; (ow * oh * 4) as usize];

    let opaque = |x: u32, y: u32| -> bool { src.rgba[((y * w + x) * 4 + 3) as usize] >= 128 };
    let set_white = |out: &mut [u8], ex: u32, ey: u32| {
        let i = ((ey * ow + ex) * 4) as usize;
        // Only fill if currently transparent (mirrors webview's `=== ''` guard).
        if out[i + 3] == 0 {
            out[i..i + 4].copy_from_slice(&WHITE);
        }
    };

    // Mark the 4 cardinal neighbours of every opaque pixel white (in the +1,+1
    // expanded grid).
    for y in 0..h {
        for x in 0..w {
            if !opaque(x, y) {
                continue;
            }
            let (ex, ey) = (x + 1, y + 1);
            set_white(&mut out, ex, ey - 1);
            set_white(&mut out, ex, ey + 1);
            set_white(&mut out, ex - 1, ey);
            set_white(&mut out, ex + 1, ey);
        }
    }

    // Clear pixels that overlap the original silhouette → hollow ring.
    for y in 0..h {
        for x in 0..w {
            if opaque(x, y) {
                let i = (((y + 1) * ow + (x + 1)) * 4) as usize;
                out[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
            }
        }
    }

    DecodedAsset { width: ow, height: oh, rgba: out }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3×3 frame with a single opaque centre pixel.
    fn dot() -> DecodedAsset {
        let mut rgba = vec![0u8; 3 * 3 * 4];
        let (row, col, w) = (1usize, 1usize, 3usize);
        let i = (row * w + col) * 4;
        rgba[i..i + 4].copy_from_slice(&[10, 20, 30, 255]);
        DecodedAsset { width: 3, height: 3, rgba }
    }

    #[test]
    fn outline_is_two_px_larger() {
        let o = outline_sprite(&dot());
        assert_eq!((o.width, o.height), (5, 5));
    }

    #[test]
    fn outline_rings_the_silhouette_and_clears_center() {
        let o = outline_sprite(&dot());
        // Centre pixel of the dot lands at expanded (2,2) → cleared (transparent).
        let c = ((2 * 5 + 2) * 4) as usize;
        assert_eq!(o.rgba[c + 3], 0, "centre cleared");
        // Its 4 cardinal neighbours are white.
        for (dx, dy) in [(0i32, -1i32), (0, 1), (-1, 0), (1, 0)] {
            let x = (2 + dx) as u32;
            let y = (2 + dy) as u32;
            let i = ((y * 5 + x) * 4) as usize;
            assert_eq!(&o.rgba[i..i + 4], &WHITE, "neighbour ({x},{y}) white");
        }
    }
}
