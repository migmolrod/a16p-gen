use crate::color::{circular_diff, oklab_to_oklch, oklab_to_srgb_u8, oklch_to_oklab, pure_hue_anchor, to_hex};
use crate::extract::{Cluster, ImageStats};
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
pub fn match_hue(clusters: &[Cluster], target_hue: f32, tolerance: f32, min_weight: f32) -> Option<[f32; 3]> {
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

/// When the image has no real presence near `target_hue`, borrow the most
/// prominent chromatic cluster and rotate it onto the target hue, keeping
/// its lightness (clamped to a workable range) and damping its chroma to
/// signal lower confidence than a genuine match.
pub fn fallback_rotate(clusters: &[Cluster], target_hue: f32, stats: &ImageStats) -> [f32; 3] {
    let prominent = clusters
        .iter()
        .filter(|c| oklab_to_oklch(c.oklab)[1] >= 0.02)
        .max_by(|a, b| a.weight.partial_cmp(&b.weight).unwrap());

    let (l, c) = match prominent {
        Some(cluster) => {
            let lch = oklab_to_oklch(cluster.oklab);
            (lch[0].clamp(0.35, 0.75), lch[1].max(0.03))
        }
        None => (stats.mean_l.clamp(0.35, 0.75), stats.mean_c.max(0.05)),
    };
    [l, (c.min(stats.mean_c.max(0.05) * 1.5)) * 0.8, target_hue]
}

pub fn resolve_hue_slot(
    slot: HueSlot,
    clusters: &[Cluster],
    stats: &ImageStats,
    tolerance: f32,
    min_weight: f32,
) -> [f32; 3] {
    let target = slot.anchor_hue();
    match_hue(clusters, target, tolerance, min_weight).unwrap_or_else(|| fallback_rotate(clusters, target, stats))
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
}

impl Default for GenParams {
    fn default() -> Self {
        Self {
            hue_tolerance: 30.0,
            min_cluster_weight: 0.02,
            chroma_clamp_factor: 1.4,
            neutral_tint_chroma: 0.015,
        }
    }
}

pub fn build_primitives(clusters: &[Cluster], stats: &ImageStats, params: &GenParams) -> Primitives {
    let chroma_cap = stats.mean_c.max(0.02) * params.chroma_clamp_factor;

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

    let neutral_hue = weighted_mean_hue(clusters);
    let neutral = generate_ramp([0.5, params.neutral_tint_chroma, neutral_hue], params.neutral_tint_chroma);

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
            is_dark: false,
        };
        let lch = fallback_rotate(&clusters, 29.0, &stats);
        assert!((lch[2] - 29.0).abs() < 1e-3);
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
