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

/// Oklab -> sRGB (0..255), naive clip-to-gamut. Good enough for a swatch
/// generator; a proper gamut mapper would reduce chroma instead of clipping,
/// left as future work if clipped colors look visibly wrong in practice.
pub fn oklab_to_srgb_u8(lab: [f32; 3]) -> [u8; 3] {
    let oklab = Oklab::new(lab[0], lab[1], lab[2]);
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
    if d > 180.0 {
        d - 360.0
    } else {
        d
    }
}

/// True Oklch hue angle (degrees) of a pure sRGB primary/secondary color.
/// Computed rather than hardcoded so the anchors are exact for whatever
/// color space conversion this crate version actually implements.
pub fn pure_hue_anchor(rgb: [u8; 3]) -> f32 {
    let lab = srgb_u8_to_oklab(rgb);
    oklab_to_oklch(lab)[2]
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
    fn primary_hue_anchors_are_distinct_and_spread() {
        let red = pure_hue_anchor([255, 0, 0]);
        let yellow = pure_hue_anchor([255, 255, 0]);
        let green = pure_hue_anchor([0, 255, 0]);
        let cyan = pure_hue_anchor([0, 255, 255]);
        let blue = pure_hue_anchor([0, 0, 255]);
        let magenta = pure_hue_anchor([255, 0, 255]);
        let hues = [red, yellow, green, cyan, blue, magenta];
        for i in 0..hues.len() {
            for j in (i + 1)..hues.len() {
                assert!(circular_diff(hues[i], hues[j]).abs() > 10.0);
            }
        }
    }
}
