use palette::convert::IntoColorUnclamped;
use palette::{FromColor, IntoColor, Oklab, Oklch, Srgb};

/// sRGB (0..255) -> Oklab (L, a, b)
pub fn srgb_u8_to_oklab(rgb: [u8; 3]) -> [f32; 3] {
    let srgb: Srgb<f32> = Srgb::new(rgb[0], rgb[1], rgb[2]).into_format();
    let oklab: Oklab = Oklab::from_color(srgb);
    [oklab.l, oklab.a, oklab.b]
}

pub fn oklab_to_oklch(lab: [f32; 3]) -> [f32; 3] {
    let oklab = Oklab::new(lab[0], lab[1], lab[2]);
    let oklch: Oklch = Oklch::from_color(oklab);
    [oklch.l, oklch.chroma, oklch.hue.into_positive_degrees()]
}

pub fn oklch_to_oklab(lch: [f32; 3]) -> [f32; 3] {
    let oklch = Oklch::new(lch[0], lch[1], lch[2]);
    let oklab: Oklab = Oklab::from_color(oklch);
    [oklab.l, oklab.a, oklab.b]
}

/// True if Oklab `lab` is inside the sRGB gamut. Uses `into_color_unclamped`
/// deliberately, not `into_color` -- `palette`'s `IntoColor`/`FromColor`
/// (the "safe" conversion traits) already clamp their output internally, so
/// checking `into_color()` output for being in `[0, 1]` is a tautology that
/// always returns true. Only the `*Unclamped` variant exposes the raw,
/// possibly out-of-range values a gamut check needs.
fn in_gamut(lab: [f32; 3]) -> bool {
    let oklab = Oklab::new(lab[0], lab[1], lab[2]);
    let srgb: Srgb<f32> = oklab.into_color_unclamped();
    let eps = 1e-4;
    srgb.red >= -eps
        && srgb.red <= 1.0 + eps
        && srgb.green >= -eps
        && srgb.green <= 1.0 + eps
        && srgb.blue >= -eps
        && srgb.blue <= 1.0 + eps
}

/// If `lch` is outside the sRGB gamut, reduce its chroma (holding L and hue
/// fixed) via binary search until it isn't. This is the standard "hold L/H,
/// back off C" gamut mapping approach -- clipping r/g/b independently
/// instead (the previous behavior) shifts both hue and perceived lightness
/// in a way that reads as visibly wrong on saturated wallpapers, since each
/// channel gets clamped by a different amount. Binary search is bounded at
/// the *original* chroma as `hi`, and correctness here leans on `in_gamut`
/// using the *unclamped* conversion -- see that function's doc comment for
/// why the obvious `into_color()` version is a tautology.
pub fn gamut_map_oklch(lch: [f32; 3]) -> [f32; 3] {
    if in_gamut(oklch_to_oklab(lch)) {
        return lch;
    }
    let mut lo = 0.0f32;
    let mut hi = lch[1];
    for _ in 0..20 {
        let mid = (lo + hi) / 2.0;
        if in_gamut(oklch_to_oklab([lch[0], mid, lch[2]])) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    [lch[0], lo, lch[2]]
}

/// Oklab -> sRGB (0..255). Gamut-maps by reducing chroma at fixed L/hue
/// (`gamut_map_oklch`) before converting, then clamps as a final safety net
/// for float rounding at the boundary (should be a no-op in practice).
pub fn oklab_to_srgb_u8(lab: [f32; 3]) -> [u8; 3] {
    let mapped_lch = gamut_map_oklch(oklab_to_oklch(lab));
    let mapped_lab = oklch_to_oklab(mapped_lch);
    let oklab = Oklab::new(mapped_lab[0], mapped_lab[1], mapped_lab[2]);
    let srgb: Srgb<f32> = oklab.into_color();
    let clamped = Srgb::new(
        srgb.red.clamp(0.0, 1.0),
        srgb.green.clamp(0.0, 1.0),
        srgb.blue.clamp(0.0, 1.0),
    );
    let out: Srgb<u8> = clamped.into_format();
    [out.red, out.green, out.blue]
}

pub fn to_hex(rgb: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}

/// Shortest signed distance from `a` to `b` on a 360-degree circle, in [-180, 180].
pub fn circular_diff(a: f32, b: f32) -> f32 {
    let d = (b - a).rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Oklch of a pure sRGB primary/secondary color. Computed rather than
/// hardcoded so it's exact for whatever this crate version's conversion
/// actually implements -- both the hue angle (used as an ANSI hue-slot
/// target) and, at the L component, the lightness that hue naturally sits
/// at when fully saturated (e.g. pure yellow's L≈0.97, pure blue's L≈0.45).
pub fn pure_corner_oklch(rgb: [u8; 3]) -> [f32; 3] {
    oklab_to_oklch(srgb_u8_to_oklab(rgb))
}

/// Interpolate from hue `a` toward hue `b` along the shorter arc, `t` in
/// [0, 1]. `t=0` stays at `a`, `t=1` lands exactly on `b`.
pub fn lerp_hue(a: f32, b: f32, t: f32) -> f32 {
    (a + circular_diff(a, b) * t).rem_euclid(360.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circular_diff_wraps_correctly() {
        assert!((circular_diff(350.0, 10.0) - 20.0).abs() < 1e-3);
        assert!((circular_diff(10.0, 350.0) + 20.0).abs() < 1e-3);
        assert!((circular_diff(0.0, 180.0) - 180.0).abs() < 1e-3);
    }

    #[test]
    fn roundtrip_oklab_oklch() {
        let lab = srgb_u8_to_oklab([200, 60, 60]);
        let lch = oklab_to_oklch(lab);
        let back = oklch_to_oklab(lch);
        assert!((lab[0] - back[0]).abs() < 1e-4);
        assert!((lab[1] - back[1]).abs() < 1e-4);
        assert!((lab[2] - back[2]).abs() < 1e-4);
    }

    #[test]
    fn gamut_map_leaves_in_gamut_colors_unchanged() {
        let lch = oklab_to_oklch(srgb_u8_to_oklab([120, 90, 200]));
        let mapped = gamut_map_oklch(lch);
        assert!((mapped[0] - lch[0]).abs() < 1e-4);
        assert!((mapped[1] - lch[1]).abs() < 1e-4);
        assert!((mapped[2] - lch[2]).abs() < 1e-4);
    }

    #[test]
    fn gamut_map_reduces_chroma_but_keeps_l_and_hue() {
        // Pure red's own max chroma at its own natural L (0.628) is
        // ~0.258 -- 0.5 is well beyond it, so the mapper must back off
        // chroma without touching L or H.
        let red = pure_corner_oklch([255, 0, 0]);
        let lch = [red[0], 0.5, red[2]];
        let mapped = gamut_map_oklch(lch);
        assert!(mapped[1] < lch[1]);
        assert!((mapped[0] - lch[0]).abs() < 1e-4);
        assert!((mapped[2] - lch[2]).abs() < 1e-4);
        assert!(in_gamut(oklch_to_oklab(mapped)));
        assert!((mapped[1] - red[1]).abs() < 0.01);
    }

    #[test]
    fn oklab_to_srgb_u8_preserves_hue_better_than_naive_clamping_would() {
        // An out-of-gamut color: naive per-channel r/g/b clamping shifts
        // hue (each channel clips by a different amount); gamut-mapped
        // chroma reduction should round-trip back to nearly the same hue.
        let red = pure_corner_oklch([255, 0, 0]);
        let lch = [red[0], 0.5, red[2]];
        let rgb = oklab_to_srgb_u8(oklch_to_oklab(lch));
        let back_hue = oklab_to_oklch(srgb_u8_to_oklab(rgb))[2];
        assert!(circular_diff(lch[2], back_hue).abs() < 2.0);
    }

    #[test]
    fn lerp_hue_endpoints() {
        assert!((lerp_hue(10.0, 100.0, 0.0) - 10.0).abs() < 1e-3);
        assert!((lerp_hue(10.0, 100.0, 1.0) - 100.0).abs() < 1e-3);
    }

    #[test]
    fn lerp_hue_takes_shorter_arc_across_the_wrap() {
        // 350 -> 10 is a 20-degree hop through 0, not a 340-degree one.
        let mid = lerp_hue(350.0, 10.0, 0.5);
        assert!((mid - 0.0).abs() < 1e-3 || (mid - 360.0).abs() < 1e-3);
    }

    #[test]
    fn primary_hue_anchors_are_distinct_and_spread() {
        let red = pure_corner_oklch([255, 0, 0])[2];
        let yellow = pure_corner_oklch([255, 255, 0])[2];
        let green = pure_corner_oklch([0, 255, 0])[2];
        let cyan = pure_corner_oklch([0, 255, 255])[2];
        let blue = pure_corner_oklch([0, 0, 255])[2];
        let magenta = pure_corner_oklch([255, 0, 255])[2];
        let hues = [red, yellow, green, cyan, blue, magenta];
        for i in 0..hues.len() {
            for j in (i + 1)..hues.len() {
                assert!(circular_diff(hues[i], hues[j]).abs() > 10.0);
            }
        }
    }
}
