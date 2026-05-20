//! Matrix-style spawn/despawn digital-rain effect (Phase 3 Day 20).
//!
//! Port of `engine/matrixEffect.ts`. Instead of drawing straight to a canvas,
//! [`render_matrix_frame`] composites the effect into a 16×32 RGBA frame which
//! the half-block rasterizer then paints — keeping the output deterministic for
//! snapshot tests (fixed seeds + fixed timer ⇒ fixed bytes).
//!
//! Per column, a bright "head" sweeps top→bottom over `MATRIX_EFFECT_DURATION`,
//! staggered by the per-column seed. Spawn reveals the character behind the
//! head; despawn consumes it. Trailing green is emitted with its source alpha;
//! at cell tiers the half-block rasterizer thresholds the sub-128 empty-space
//! trail out (only the head + revealed char pixels + green overlay show), so the
//! cell-tier effect is a faithful-but-reduced version of the canvas original.
//! Image tiers (which can alpha-composite over the floor) get the full data.

use rand::Rng;

use crate::assets::DecodedAsset;
use crate::office::types::{
    MatrixEffectKind, MATRIX_COLUMN_STAGGER_RANGE, MATRIX_EFFECT_DURATION, MATRIX_FLICKER_FPS,
    MATRIX_FLICKER_VISIBILITY_THRESHOLD, MATRIX_SPRITE_COLS, MATRIX_SPRITE_ROWS,
    MATRIX_TRAIL_DIM_THRESHOLD, MATRIX_TRAIL_EMPTY_ALPHA, MATRIX_TRAIL_LENGTH,
    MATRIX_TRAIL_MID_THRESHOLD, MATRIX_TRAIL_OVERLAY_ALPHA,
};

/// Generate 16 per-column random seed values controlling per-column stagger.
pub fn matrix_effect_seeds(rng: &mut impl Rng) -> [f32; 16] {
    let mut seeds = [0.0f32; 16];
    for s in &mut seeds {
        *s = rng.gen();
    }
    seeds
}

const HEAD: [u8; 3] = [0xcc, 0xff, 0xcc];
const GREEN_BRIGHT: [u8; 3] = [0, 255, 65];
const GREEN_MID: [u8; 3] = [0, 170, 40];
const GREEN_DIM: [u8; 3] = [0, 85, 20];

/// Hash-based flicker: ~70% visible (`hash < 180`). Mirrors `flickerVisible`.
fn flicker_visible(col: usize, row: usize, time: f32) -> bool {
    let t = (time * MATRIX_FLICKER_FPS).floor() as i64;
    let hash = ((col as i64 * 7 + row as i64 * 13 + t * 31) & 0xff) as u32;
    hash < MATRIX_FLICKER_VISIBILITY_THRESHOLD
}

/// Alpha-blend `src` over opaque `dst` with `a` in 0..1 → opaque result.
fn over(dst: [u8; 3], src: [u8; 3], a: f32) -> [u8; 3] {
    let a = a.clamp(0.0, 1.0);
    [
        (dst[0] as f32 * (1.0 - a) + src[0] as f32 * a).round() as u8,
        (dst[1] as f32 * (1.0 - a) + src[1] as f32 * a).round() as u8,
        (dst[2] as f32 * (1.0 - a) + src[2] as f32 * a).round() as u8,
    ]
}

/// Composite the matrix effect for `base` (a 16×32 character frame) at the given
/// effect kind/timer/seeds into a fresh 16×32 RGBA frame.
pub fn render_matrix_frame(
    base: &DecodedAsset,
    kind: MatrixEffectKind,
    timer: f32,
    seeds: &[f32; 16],
) -> DecodedAsset {
    let w = base.width;
    let h = base.height;
    let mut out = vec![0u8; (w * h * 4) as usize];

    let progress = timer / MATRIX_EFFECT_DURATION;
    let is_spawn = kind == MatrixEffectKind::Spawn;
    let total_sweep = MATRIX_SPRITE_ROWS as f32 + MATRIX_TRAIL_LENGTH;

    let put = |out: &mut [u8], row: usize, col: usize, rgb: [u8; 3], a: u8| {
        let i = ((row as u32 * w + col as u32) * 4) as usize;
        out[i] = rgb[0];
        out[i + 1] = rgb[1];
        out[i + 2] = rgb[2];
        out[i + 3] = a;
    };

    let cols = (MATRIX_SPRITE_COLS as u32).min(w) as usize;
    let rows = (MATRIX_SPRITE_ROWS as u32).min(h) as usize;

    for col in 0..cols {
        let stagger = seeds.get(col).copied().unwrap_or(0.0) * MATRIX_COLUMN_STAGGER_RANGE;
        let col_progress =
            ((progress - stagger) / (1.0 - MATRIX_COLUMN_STAGGER_RANGE)).clamp(0.0, 1.0);
        let head_row = col_progress * total_sweep;

        for row in 0..rows {
            let bi = ((row as u32 * w + col as u32) * 4) as usize;
            let pixel = [base.rgba[bi], base.rgba[bi + 1], base.rgba[bi + 2]];
            let has_pixel = base.rgba[bi + 3] >= 128;
            let dist = head_row - row as f32;

            if is_spawn {
                if dist < 0.0 {
                    // above head: invisible
                } else if dist < 1.0 {
                    put(&mut out, row, col, HEAD, 255);
                } else if dist < MATRIX_TRAIL_LENGTH {
                    let trail_pos = dist / MATRIX_TRAIL_LENGTH;
                    if has_pixel {
                        let mut rgb = pixel;
                        if flicker_visible(col, row, timer) {
                            let a = (1.0 - trail_pos) * MATRIX_TRAIL_OVERLAY_ALPHA;
                            rgb = over(pixel, GREEN_BRIGHT, a);
                        }
                        put(&mut out, row, col, rgb, 255);
                    } else if flicker_visible(col, row, timer) {
                        let a = (1.0 - trail_pos) * MATRIX_TRAIL_EMPTY_ALPHA;
                        let rgb = trail_color(trail_pos);
                        put(&mut out, row, col, rgb, (a * 255.0).round() as u8);
                    }
                } else if has_pixel {
                    put(&mut out, row, col, pixel, 255);
                }
            } else {
                // despawn
                if dist < 0.0 {
                    if has_pixel {
                        put(&mut out, row, col, pixel, 255);
                    }
                } else if dist < 1.0 {
                    put(&mut out, row, col, HEAD, 255);
                } else if dist < MATRIX_TRAIL_LENGTH && flicker_visible(col, row, timer) {
                    let trail_pos = dist / MATRIX_TRAIL_LENGTH;
                    let a = (1.0 - trail_pos) * MATRIX_TRAIL_EMPTY_ALPHA;
                    let rgb = trail_color(trail_pos);
                    put(&mut out, row, col, rgb, (a * 255.0).round() as u8);
                }
                // below trail: consumed (transparent)
            }
        }
    }

    DecodedAsset { width: w, height: h, rgba: out }
}

fn trail_color(trail_pos: f32) -> [u8; 3] {
    if trail_pos < MATRIX_TRAIL_MID_THRESHOLD {
        GREEN_BRIGHT
    } else if trail_pos < MATRIX_TRAIL_DIM_THRESHOLD {
        GREEN_MID
    } else {
        GREEN_DIM
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_frame() -> DecodedAsset {
        // 16×32 fully-opaque white frame.
        DecodedAsset { width: 16, height: 32, rgba: vec![255u8; 16 * 32 * 4] }
    }

    #[test]
    fn spawn_start_top_revealed_bottom_hidden() {
        // At t≈0 the head is at the top; top rows show (head/near), bottom rows
        // (below head, above their reveal) are still invisible for spawn.
        let f = render_matrix_frame(&solid_frame(), MatrixEffectKind::Spawn, 0.0, &[0.0; 16]);
        // Bottom row of the animated region, col 0: dist = headRow(~0) - 23 < 0 → invisible.
        let i = (23 * 16 * 4) as usize; // row 23, col 0
        assert_eq!(f.rgba[i + 3], 0, "bottom invisible at spawn start");
    }

    #[test]
    fn spawn_end_fully_revealed() {
        // At completion the head has swept past; all opaque base pixels show.
        let f = render_matrix_frame(
            &solid_frame(),
            MatrixEffectKind::Spawn,
            MATRIX_EFFECT_DURATION,
            &[0.0; 16],
        );
        let i = ((10 * 16 + 5) * 4) as usize;
        assert_eq!(f.rgba[i + 3], 255, "revealed at spawn end");
        assert_eq!(&f.rgba[i..i + 3], &[255, 255, 255], "shows base pixel");
    }

    #[test]
    fn despawn_end_consumed() {
        let f = render_matrix_frame(
            &solid_frame(),
            MatrixEffectKind::Despawn,
            MATRIX_EFFECT_DURATION,
            &[0.0; 16],
        );
        // Everything consumed → top region transparent.
        let i = ((2 * 16 + 2) * 4) as usize;
        assert_eq!(f.rgba[i + 3], 0, "consumed at despawn end");
    }

    #[test]
    fn deterministic_for_fixed_seeds_and_timer() {
        let a = render_matrix_frame(&solid_frame(), MatrixEffectKind::Spawn, 0.15, &[0.5; 16]);
        let b = render_matrix_frame(&solid_frame(), MatrixEffectKind::Spawn, 0.15, &[0.5; 16]);
        assert_eq!(a.rgba, b.rgba);
    }

    #[test]
    fn flicker_threshold_matches_webview_hash() {
        // hash = (col*7 + row*13 + t*31) & 0xff < 180. At t=0, col=0,row=0 → 0 < 180.
        assert!(flicker_visible(0, 0, 0.0));
    }
}
