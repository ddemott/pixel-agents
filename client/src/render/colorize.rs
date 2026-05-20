//! HSL "Adjust" colour transform — Rust port of `colorize.ts`'s `adjustSprite`
//! (Day 18). Used for character hue shifts: shift each opaque pixel's hue (and
//! optionally saturation / brightness / contrast) in HSL space.
//!
//! The webview's Photoshop-style "Colorize" mode (grayscale → fixed HSL, used
//! for floor tiles) is intentionally not ported here — characters only ever use
//! Adjust mode (`{ h: hueShift, s: 0, b: 0, c: 0 }`), see `spriteData.ts`'s
//! `hueShiftSprites`. Floor colourization arrives with the floor asset path.

/// Convert RGB (0-255) → HSL (`h` 0-360, `s` 0-1, `l` 0-1). Mirrors `rgbToHsl`.
pub fn rgb_to_hsl(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let l = (max + min) / 2.0;
    if (max - min).abs() < f32::EPSILON {
        return (0.0, 0.0, l);
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if (max - rf).abs() < f32::EPSILON {
        ((gf - bf) / d + if gf < bf { 6.0 } else { 0.0 }) * 60.0
    } else if (max - gf).abs() < f32::EPSILON {
        ((bf - rf) / d + 2.0) * 60.0
    } else {
        ((rf - gf) / d + 4.0) * 60.0
    };
    (h, s, l)
}

/// Convert HSL (`h` 0-360, `s` 0-1, `l` 0-1) → RGB (0-255). Mirrors `hslToHex`.
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = h / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = if hp < 1.0 {
        (c, x, 0.0)
    } else if hp < 2.0 {
        (x, c, 0.0)
    } else if hp < 3.0 {
        (0.0, c, x)
    } else if hp < 4.0 {
        (0.0, x, c)
    } else if hp < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };
    let m = l - c / 2.0;
    (
        clamp255((r1 + m) * 255.0),
        clamp255((g1 + m) * 255.0),
        clamp255((b1 + m) * 255.0),
    )
}

fn clamp255(v: f32) -> u8 {
    v.round().clamp(0.0, 255.0) as u8
}

/// Adjust packed RGBA8 in place by shifting HSL. `h_shift` rotates hue (degrees),
/// `s_shift` shifts saturation (-100..100), `b` shifts lightness (-100..100), `c`
/// adjusts contrast around 0.5 (-100..100). Fully-transparent pixels are left
/// untouched. Mirrors `adjustSprite` semantics exactly.
pub fn adjust_rgba(rgba: &mut [u8], h_shift: f32, s_shift: f32, b: f32, c: f32) {
    for px in rgba.chunks_exact_mut(4) {
        if px[3] == 0 {
            continue; // transparent — webview skips empty cells
        }
        let (orig_h, orig_s, orig_l) = rgb_to_hsl(px[0], px[1], px[2]);

        let new_h = (((orig_h + h_shift) % 360.0) + 360.0) % 360.0;
        let new_s = (orig_s + s_shift / 100.0).clamp(0.0, 1.0);

        let mut lightness = orig_l;
        if c != 0.0 {
            let factor = (100.0 + c) / 100.0;
            lightness = 0.5 + (lightness - 0.5) * factor;
        }
        if b != 0.0 {
            lightness += b / 200.0;
        }
        lightness = lightness.clamp(0.0, 1.0);

        let (nr, ng, nb) = hsl_to_rgb(new_h, new_s, lightness);
        px[0] = nr;
        px[1] = ng;
        px[2] = nb;
        // alpha (px[3]) preserved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_roundtrips_primary_colours() {
        for rgb in [[200u8, 40, 40], [40, 200, 40], [40, 40, 200], [123, 77, 200]] {
            let (h, s, l) = rgb_to_hsl(rgb[0], rgb[1], rgb[2]);
            let (r, g, b) = hsl_to_rgb(h, s, l);
            // Round-trip within ±1 per channel (rounding).
            assert!((r as i32 - rgb[0] as i32).abs() <= 1);
            assert!((g as i32 - rgb[1] as i32).abs() <= 1);
            assert!((b as i32 - rgb[2] as i32).abs() <= 1);
        }
    }

    #[test]
    fn grayscale_has_zero_saturation() {
        let (_, s, l) = rgb_to_hsl(128, 128, 128);
        assert_eq!(s, 0.0);
        assert!((l - 0.5).abs() < 0.01);
    }

    #[test]
    fn hue_shift_360_is_identity() {
        let mut a = [200u8, 40, 40, 255];
        let orig = a;
        adjust_rgba(&mut a, 360.0, 0.0, 0.0, 0.0);
        for i in 0..4 {
            assert!((a[i] as i32 - orig[i] as i32).abs() <= 1, "channel {i}");
        }
    }

    #[test]
    fn hue_shift_180_changes_colour_keeps_alpha() {
        let mut a = [200u8, 40, 40, 200];
        adjust_rgba(&mut a, 180.0, 0.0, 0.0, 0.0);
        assert_eq!(a[3], 200, "alpha preserved");
        // A 180° rotation of a saturated red moves it well away from red.
        assert_ne!([a[0], a[1], a[2]], [200, 40, 40]);
    }

    #[test]
    fn transparent_pixels_untouched() {
        let mut a = [200u8, 40, 40, 0];
        adjust_rgba(&mut a, 90.0, 0.0, 0.0, 0.0);
        assert_eq!(a, [200, 40, 40, 0]);
    }
}
