//! Speech-bubble sprites (Phase 3 Day 19).
//!
//! Rust copies of `bubble-permission.json` / `bubble-waiting.json` (11×13 each):
//! permission = white box with amber "..."; waiting = white box with a green
//! checkmark; both have a downward tail pointer. Built once into RGBA
//! [`DecodedAsset`]s so the half-block rasterizer can paint them above a head.
//!
//! Webview fades the waiting bubble over its last `BUBBLE_FADE_DURATION_SEC`.
//! Cell tiers can't alpha-blend per pixel (bg/fg are solid), so the fade is
//! **skipped** here — the bubble stays solid until `state.rs` clears it. The
//! real fade lands when the image tiers (which can alpha-composite) go live.

use std::sync::OnceLock;

use crate::assets::DecodedAsset;
use crate::office::types::BubbleType;

const W: u32 = 11;
const H: u32 = 13;

// Pixel grids verbatim from the JSON sprites. '_' = transparent.
const PERMISSION_ROWS: [&str; 13] = [
    "BBBBBBBBBBB",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "BFFAFAFAFFB",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "BBBBBBBBBBB",
    "____BBB____",
    "_____B_____",
    "___________",
];
const WAITING_ROWS: [&str; 13] = [
    "_BBBBBBBBB_",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "BFFFFFFFGFB",
    "BFFFFFFGFFB",
    "BFFGFFGFFFB",
    "BFFFGGFFFFB",
    "BFFFFFFFFFB",
    "BFFFFFFFFFB",
    "_BBBBBBBBB_",
    "____BBB____",
    "_____B_____",
    "___________",
];

/// Map a palette key to RGBA. Keys shared across both sprites where they match.
fn key_rgba(k: u8) -> [u8; 4] {
    match k {
        b'B' => [0x55, 0x55, 0x66, 255], // border
        b'F' => [0xEE, 0xEE, 0xFF, 255], // fill
        b'A' => [0xCC, 0xA7, 0x00, 255], // amber dots
        b'G' => [0x44, 0xBB, 0x66, 255], // green check
        _ => [0, 0, 0, 0],               // '_' transparent
    }
}

fn build(rows: &[&str; 13]) -> DecodedAsset {
    let mut rgba = vec![0u8; (W * H * 4) as usize];
    for (r, row) in rows.iter().enumerate() {
        for (c, ch) in row.bytes().enumerate() {
            let i = ((r as u32 * W + c as u32) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&key_rgba(ch));
        }
    }
    DecodedAsset { width: W, height: H, rgba }
}

/// The bubble sprite for a [`BubbleType`] (built once, reused).
pub fn bubble_sprite(kind: BubbleType) -> &'static DecodedAsset {
    static PERMISSION: OnceLock<DecodedAsset> = OnceLock::new();
    static WAITING: OnceLock<DecodedAsset> = OnceLock::new();
    match kind {
        BubbleType::Permission => PERMISSION.get_or_init(|| build(&PERMISSION_ROWS)),
        BubbleType::Waiting => WAITING.get_or_init(|| build(&WAITING_ROWS)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_dims_and_amber_dot() {
        let s = bubble_sprite(BubbleType::Permission);
        assert_eq!((s.width, s.height), (11, 13));
        // Row 5, col 3 is the first amber dot ('A').
        let i = ((5 * 11 + 3) * 4) as usize;
        assert_eq!(&s.rgba[i..i + 4], &[0xCC, 0xA7, 0x00, 255]);
    }

    #[test]
    fn waiting_has_green_check_and_transparent_corners() {
        let s = bubble_sprite(BubbleType::Waiting);
        // (0,0) corner is '_' → transparent.
        assert_eq!(&s.rgba[0..4], &[0, 0, 0, 0]);
        // Row 3, col 8 is a green ('G') checkmark pixel.
        let i = ((3 * 11 + 8) * 4) as usize;
        assert_eq!(&s.rgba[i..i + 4], &[0x44, 0xBB, 0x66, 255]);
    }

    #[test]
    fn sprites_are_cached_singletons() {
        let a = bubble_sprite(BubbleType::Permission);
        let b = bubble_sprite(BubbleType::Permission);
        assert!(std::ptr::eq(a, b));
    }
}
