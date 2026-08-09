use crate::color::{
    circular_diff, lerp_hue, oklab_to_oklch, oklab_to_srgb_u8, oklch_to_oklab, pure_hue_anchor,
    to_hex,
};
use crate::extract::{CHROMATIC_THRESHOLD, Cluster, ImageStats};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize)]
pub struct Swatch {
    pub hex: String,
    pub rgb: [u8; 3],
    pub oklch: [f32; 3],
}

fn swatch_from_oklch(lch: [f32; 3]) -> Swatch {
    let lab = oklch_to_oklab(lch);
    let rgb = oklab_to_srgb_u8(lab);
    Swatch {
        hex: to_hex(rgb),
        rgb,
        oklch: lch,
    }
}

pub type Ramp = BTreeMap<u16, Swatch>;

#[derive(Debug, Clone, Copy)]
pub enum HueSlot {
    Red,
    Yellow,
    Green,
    Cyan,
    Blue,
    Magenta,
}

impl HueSlot {
    pub const ALL: [HueSlot; 6] = [
        HueSlot::Red,
        HueSlot::Yellow,
        HueSlot::Green,
        HueSlot::Cyan,
        HueSlot::Blue,
        HueSlot::Magenta,
    ];

    pub fn name(self) -> &'static str {
        match self {
            HueSlot::Red => "red",
            HueSlot::Yellow => "yellow",
            HueSlot::Green => "green",
            HueSlot::Cyan => "cyan",
            HueSlot::Blue => "blue",
            HueSlot::Magenta => "magenta",
        }
    }

    /// True Oklch hue angle of the pure sRGB primary/secondary, computed
    /// (not hardcoded) so it tracks whatever this crate version's conversion
    /// actually produces.
    pub fn anchor_hue(self) -> f32 {
        let rgb = match self {
            HueSlot::Red => [255, 0, 0],
            HueSlot::Yellow => [255, 255, 0],
            HueSlot::Green => [0, 255, 0],
            HueSlot::Cyan => [0, 255, 255],
            HueSlot::Blue => [0, 0, 255],
            HueSlot::Magenta => [255, 0, 255],
        };
        pure_hue_anchor(rgb)
    }
}

/// Nearest weighted cluster to `target_hue` within `tolerance` degrees and
/// carrying at least `min_weight` share of the image. Skips near-neutral
/// clusters since a gray pixel's hue angle is noise, not signal.
pub fn match_hue(
    clusters: &[Cluster],
    target_hue: f32,
    tolerance: f32,
    min_weight: f32,
) -> Option<[f32; 3]> {
    let mut best: Option<(f32, [f32; 3])> = None;
    for c in clusters {
        if c.weight < min_weight {
            continue;
        }
        let lch = oklab_to_oklch(c.oklab);
        if lch[1] < 0.01 {
            continue;
        }
        let d = circular_diff(target_hue, lch[2]).abs();
        if d <= tolerance && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, lch));
        }
    }
    best.map(|(_, lch)| lch)
}

/// The most *vivid* cluster clearing `min_weight` -- the same "real signal,
/// not noise" floor `match_hue` uses -- rather than the most prevalent one.
/// A small but vivid region (a game sprite's highlight, a lantern glow) is
/// a stronger color signal than a large but barely-chromatic one (fog, a
/// muted background wash) once both have cleared the noise floor; picking
/// by weight alone would systematically prefer the dull, sprawling cluster
/// over the vivid, compact one -- exactly backwards from what reads as an
/// "accent" color. Weighting by `weight * chroma` instead of a hard floor
/// was tried and rejected: a 5x weight gap still beats a 3x chroma gap
/// under that product, so it doesn't reliably favor vividness either.
/// Shared by `fallback_rotate` and `pick_vivid_accent`.
fn most_vivid(clusters: &[Cluster], min_weight: f32) -> Option<Cluster> {
    clusters
        .iter()
        .copied()
        .filter(|c| c.weight >= min_weight && oklab_to_oklch(c.oklab)[1] >= CHROMATIC_THRESHOLD)
        .max_by(|a, b| {
            oklab_to_oklch(a.oklab)[1]
                .partial_cmp(&oklab_to_oklch(b.oklab)[1])
                .unwrap()
        })
}

/// When the image has no real presence near `target_hue`, borrow the most
/// vivid qualifying cluster (see `most_vivid`) and rotate it onto the
/// target hue, keeping its lightness (clamped to a workable range) and
/// damping its chroma to signal lower confidence than a genuine match.
pub fn fallback_rotate(
    clusters: &[Cluster],
    target_hue: f32,
    stats: &ImageStats,
    min_weight: f32,
) -> [f32; 3] {
    let vivid = most_vivid(clusters, min_weight);

    let (l, c) = match vivid {
        Some(cluster) => {
            let lch = oklab_to_oklch(cluster.oklab);
            (lch[0].clamp(0.35, 0.75), lch[1].max(0.03))
        }
        None => (stats.mean_l.clamp(0.35, 0.75), stats.mean_c.max(0.05)),
    };
    [
        l,
        (c.min(stats.chromatic_mean_c.max(0.05) * 1.5)) * 0.8,
        target_hue,
    ]
}

pub fn resolve_hue_slot(
    slot: HueSlot,
    clusters: &[Cluster],
    stats: &ImageStats,
    tolerance: f32,
    min_weight: f32,
) -> [f32; 3] {
    let target = slot.anchor_hue();
    match_hue(clusters, target, tolerance, min_weight)
        .unwrap_or_else(|| fallback_rotate(clusters, target, stats, min_weight))
}

/// Weighted circular mean hue across clusters, weighted by weight*chroma so
/// near-gray clusters (meaningless hue) don't skew the estimate.
pub fn weighted_mean_hue(clusters: &[Cluster]) -> f32 {
    let mut x = 0.0f32;
    let mut y = 0.0f32;
    for c in clusters {
        let lch = oklab_to_oklch(c.oklab);
        let w = c.weight * lch[1];
        let rad = lch[2].to_radians();
        x += w * rad.cos();
        y += w * rad.sin();
    }
    if x.abs() < 1e-6 && y.abs() < 1e-6 {
        0.0
    } else {
        y.atan2(x).to_degrees().rem_euclid(360.0)
    }
}

pub fn pick_accent(clusters: &[Cluster]) -> [f32; 3] {
    clusters
        .iter()
        .max_by(|a, b| a.weight.partial_cmp(&b.weight).unwrap())
        .map(|c| oklab_to_oklch(c.oklab))
        .expect("clusters is non-empty")
}

/// The wallpaper's "defining" color for tinting bg/fg: most vivid cluster
/// clearing `min_weight` (see `most_vivid`), not most prevalent. Deliberately
/// separate from `pick_accent` -- that one stays prevalence-based (matugen-
/// equivalent) for the `accent`/`primary` token, since changing its meaning
/// would be a breaking change to an already-documented contract. Falls back
/// to `pick_accent` if nothing clears the chromatic threshold, so a
/// genuinely near-monochrome image still gets *a* hue rather than none.
pub fn pick_vivid_accent(clusters: &[Cluster], min_weight: f32) -> [f32; 3] {
    most_vivid(clusters, min_weight)
        .map(|c| oklab_to_oklch(c.oklab))
        .unwrap_or_else(|| pick_accent(clusters))
}

pub const RAMP_STEPS: [u16; 11] = [50, 100, 200, 300, 400, 500, 600, 700, 800, 900, 950];

fn lightness_for_step(step: u16) -> f32 {
    let t = step as f32 / 950.0;
    0.95 - t * 0.80
}

/// Damp chroma toward the extremes of the lightness ramp so very light/dark
/// steps don't rely on out-of-gamut chroma that gets naively clipped.
fn chroma_taper(l: f32) -> f32 {
    let d = (l - 0.5).abs() * 2.0;
    (1.0 - d * 0.85).clamp(0.15, 1.0)
}

pub fn generate_ramp(anchor_lch: [f32; 3], chroma_cap: f32) -> Ramp {
    let hue = anchor_lch[2];
    let base_chroma = anchor_lch[1].min(chroma_cap);
    RAMP_STEPS
        .iter()
        .map(|&step| {
            let l = lightness_for_step(step);
            let c = base_chroma * chroma_taper(l);
            (step, swatch_from_oklch([l, c, hue]))
        })
        .collect()
}

#[derive(Serialize)]
pub struct Primitives {
    pub red: Ramp,
    pub yellow: Ramp,
    pub green: Ramp,
    pub cyan: Ramp,
    pub blue: Ramp,
    pub magenta: Ramp,
    pub neutral: Ramp,
    pub accent: Ramp,
}

impl Primitives {
    pub fn get_ramp(&self, name: &str) -> Option<&Ramp> {
        match name {
            "red" => Some(&self.red),
            "yellow" => Some(&self.yellow),
            "green" => Some(&self.green),
            "cyan" => Some(&self.cyan),
            "blue" => Some(&self.blue),
            "magenta" => Some(&self.magenta),
            "neutral" => Some(&self.neutral),
            "accent" => Some(&self.accent),
            _ => None,
        }
    }
}

pub struct GenParams {
    pub hue_tolerance: f32,
    pub min_cluster_weight: f32,
    pub chroma_clamp_factor: f32,
    pub neutral_tint_chroma: f32,
    pub neutral_accent_influence: f32,
}

impl Default for GenParams {
    fn default() -> Self {
        Self {
            hue_tolerance: 30.0,
            // Low enough that a small-but-deliberate accent region (a
            // sprite highlight, a lantern glow -- a few hundred pixels in a
            // downsampled image) still counts as signal. k-means already
            // absorbs single-pixel/anti-aliasing noise into larger
            // clusters, so this doesn't need to be a big floor.
            min_cluster_weight: 0.005,
            chroma_clamp_factor: 1.4,
            neutral_tint_chroma: 0.015,
            // 0 = bg/fg stay flat gray (matches pre-this-field behavior
            // exactly). Opt-in, not on by default: this is a judgment call
            // about how much personality the background should have, and
            // going too high reintroduces matugen's "everything is one
            // hue" problem, just relocated to bg/fg.
            neutral_accent_influence: 0.0,
        }
    }
}

pub fn build_primitives(
    clusters: &[Cluster],
    stats: &ImageStats,
    params: &GenParams,
) -> Primitives {
    // chromatic_mean_c (not mean_c): a mostly-dark/desaturated image
    // shouldn't have its accent vividness crushed by pixels that are
    // achromatic in the first place -- see ImageStats::chromatic_mean_c.
    let chroma_cap = stats.chromatic_mean_c.max(0.02) * params.chroma_clamp_factor;

    let mut ramps: BTreeMap<&'static str, Ramp> = BTreeMap::new();
    for slot in HueSlot::ALL {
        let anchor = resolve_hue_slot(
            slot,
            clusters,
            stats,
            params.hue_tolerance,
            params.min_cluster_weight,
        );
        ramps.insert(slot.name(), generate_ramp(anchor, chroma_cap));
    }

    // bg/fg tint: blend from the flat average-hue/fixed-chroma baseline
    // (influence=0, original behavior) toward the wallpaper's vivid accent
    // (influence=1) -- see pick_vivid_accent's doc comment for why "vivid"
    // and not "prevalent" is the right notion of accent here.
    let base_hue = weighted_mean_hue(clusters);
    let influence = params.neutral_accent_influence.clamp(0.0, 1.0);
    let vivid_accent = pick_vivid_accent(clusters, params.min_cluster_weight);
    let neutral_hue = lerp_hue(base_hue, vivid_accent[2], influence);
    let vivid_chroma = vivid_accent[1].min(chroma_cap);
    let neutral_chroma = params.neutral_tint_chroma
        + (vivid_chroma - params.neutral_tint_chroma).max(0.0) * influence;
    let neutral = generate_ramp([0.5, neutral_chroma, neutral_hue], neutral_chroma);

    let accent_anchor = pick_accent(clusters);
    let accent = generate_ramp(accent_anchor, chroma_cap.max(accent_anchor[1]));

    Primitives {
        red: ramps.remove("red").unwrap(),
        yellow: ramps.remove("yellow").unwrap(),
        green: ramps.remove("green").unwrap(),
        cyan: ramps.remove("cyan").unwrap(),
        blue: ramps.remove("blue").unwrap(),
        magenta: ramps.remove("magenta").unwrap(),
        neutral,
        accent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_monotonically_lighter_toward_low_steps() {
        let ramp = generate_ramp([0.5, 0.15, 30.0], 0.2);
        let l50 = ramp[&50].oklch[0];
        let l950 = ramp[&950].oklch[0];
        assert!(l50 > l950);
    }

    #[test]
    fn ramp_chroma_never_exceeds_cap() {
        let ramp = generate_ramp([0.5, 0.5, 30.0], 0.2);
        for swatch in ramp.values() {
            assert!(swatch.oklch[1] <= 0.2 + 1e-4);
        }
    }

    #[test]
    fn match_hue_finds_close_cluster() {
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.2, 29.0]),
                weight: 0.5,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.2, 250.0]),
                weight: 0.5,
            },
        ];
        let found = match_hue(&clusters, 29.0, 30.0, 0.01).unwrap();
        assert!((found[2] - 29.0).abs() < 1.0);
    }

    #[test]
    fn match_hue_respects_tolerance() {
        let clusters = vec![Cluster {
            oklab: oklch_to_oklab([0.5, 0.2, 100.0]),
            weight: 0.5,
        }];
        assert!(match_hue(&clusters, 29.0, 10.0, 0.01).is_none());
    }

    #[test]
    fn fallback_rotate_lands_exactly_on_target_hue() {
        let clusters = vec![Cluster {
            oklab: oklch_to_oklab([0.6, 0.15, 100.0]),
            weight: 0.9,
        }];
        let stats = ImageStats {
            mean_l: 0.5,
            mean_c: 0.1,
            chromatic_mean_c: 0.1,
            is_dark: false,
        };
        let lch = fallback_rotate(&clusters, 29.0, &stats, 0.02);
        assert!((lch[2] - 29.0).abs() < 1e-3);
    }

    #[test]
    fn fallback_rotate_prefers_vivid_small_cluster_over_dull_prevalent_one() {
        // A large muted "fog" cluster and a small vivid "accent" cluster,
        // both above the min_weight noise floor -- the accent should win on
        // chroma even though it covers far fewer pixels.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.3, 0.03, 100.0]),
                weight: 0.3,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.1, 100.0]),
                weight: 0.01,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.3,
            mean_c: 0.02,
            chromatic_mean_c: 0.04,
            is_dark: true,
        };
        let lch = fallback_rotate(&clusters, 29.0, &stats, 0.005);
        // Rotated chroma is damped/capped, but should still track the vivid
        // cluster's L (0.5), not the dull cluster's L (0.3).
        assert!((lch[0] - 0.5).abs() < 1e-3);
    }

    #[test]
    fn fallback_rotate_ignores_vivid_cluster_below_min_weight() {
        // A tiny, spuriously-vivid cluster (compression artifact, a couple
        // stray pixels) shouldn't outrank a properly-sized chromatic
        // cluster just because it's more saturated.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.3, 0.03, 100.0]),
                weight: 0.3,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.1, 100.0]),
                weight: 0.001,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.3,
            mean_c: 0.02,
            chromatic_mean_c: 0.03,
            is_dark: true,
        };
        let lch = fallback_rotate(&clusters, 29.0, &stats, 0.005);
        // L=0.3 clamped up to the 0.35 floor -- the dull cluster's L, not
        // the vivid-but-too-small cluster's L=0.5.
        assert!((lch[0] - 0.35).abs() < 1e-3);
    }

    #[test]
    fn pick_vivid_accent_prefers_vivid_over_prevalent() {
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.3, 0.03, 100.0]),
                weight: 0.5,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.1, 40.0]),
                weight: 0.02,
            },
        ];
        let lch = pick_vivid_accent(&clusters, 0.005);
        assert!((lch[2] - 40.0).abs() < 1e-3);
    }

    #[test]
    fn pick_vivid_accent_falls_back_to_pick_accent_when_fully_achromatic() {
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.3, 0.0, 0.0]),
                weight: 0.7,
            },
            Cluster {
                oklab: oklch_to_oklab([0.6, 0.0, 0.0]),
                weight: 0.3,
            },
        ];
        let lch = pick_vivid_accent(&clusters, 0.005);
        assert!((lch[0] - 0.3).abs() < 1e-3);
    }

    #[test]
    fn neutral_accent_influence_zero_matches_flat_baseline() {
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.15, 40.0]),
                weight: 0.05,
            },
            Cluster {
                oklab: oklch_to_oklab([0.2, 0.0, 0.0]),
                weight: 0.95,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.25,
            mean_c: 0.01,
            chromatic_mean_c: 0.15,
            is_dark: true,
        };
        let params = GenParams {
            neutral_accent_influence: 0.0,
            ..GenParams::default()
        };
        let primitives = build_primitives(&clusters, &stats, &params);
        let n500 = &primitives.neutral[&500];
        let expected = params.neutral_tint_chroma * chroma_taper(lightness_for_step(500));
        assert!((n500.oklch[1] - expected).abs() < 1e-4);
    }

    #[test]
    fn neutral_accent_influence_one_tracks_vivid_accent_hue() {
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.15, 40.0]),
                weight: 0.05,
            },
            Cluster {
                oklab: oklch_to_oklab([0.2, 0.0, 0.0]),
                weight: 0.95,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.25,
            mean_c: 0.01,
            chromatic_mean_c: 0.15,
            is_dark: true,
        };
        let params = GenParams {
            neutral_accent_influence: 1.0,
            ..GenParams::default()
        };
        let primitives = build_primitives(&clusters, &stats, &params);
        let n500 = &primitives.neutral[&500];
        assert!(circular_diff(n500.oklch[2], 40.0).abs() < 1.0);
        assert!(n500.oklch[1] > params.neutral_tint_chroma);
    }

    #[test]
    fn all_six_hue_anchors_are_far_apart() {
        for a in HueSlot::ALL {
            for b in HueSlot::ALL {
                if a.name() == b.name() {
                    continue;
                }
                assert!(circular_diff(a.anchor_hue(), b.anchor_hue()).abs() > 10.0);
            }
        }
    }
}
