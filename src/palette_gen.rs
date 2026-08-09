use crate::color::{
    circular_diff, lerp_hue, oklab_to_oklch, oklab_to_srgb_u8, oklch_to_oklab, pure_corner_oklch,
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

    fn corner_rgb(self) -> [u8; 3] {
        match self {
            HueSlot::Red => [255, 0, 0],
            HueSlot::Yellow => [255, 255, 0],
            HueSlot::Green => [0, 255, 0],
            HueSlot::Cyan => [0, 255, 255],
            HueSlot::Blue => [0, 0, 255],
            HueSlot::Magenta => [255, 0, 255],
        }
    }

    /// Oklch of this slot's pure sRGB primary/secondary, computed (not
    /// hardcoded) so it tracks whatever this crate version's conversion
    /// actually produces. The L component is this hue's "natural" fully-
    /// saturated lightness (pure yellow's L≈0.97, pure blue's L≈0.45) --
    /// used by `peak_lightness_for_hue` to bend the ramp's lightness curve.
    pub fn anchor_lch(self) -> [f32; 3] {
        pure_corner_oklch(self.corner_rgb())
    }

    /// True Oklch hue angle of the pure sRGB primary/secondary.
    pub fn anchor_hue(self) -> f32 {
        self.anchor_lch()[2]
    }
}

/// Estimate the lightness at which an arbitrary hue "naturally" reaches its
/// highest chroma, by interpolating between the two nearest of the 6 known
/// pure-primary/secondary corners (whose exact L is computed, not
/// hardcoded -- see `HueSlot::anchor_lch`). A true per-hue gamut-boundary
/// search (binary-search max in-gamut chroma per L, then hunt for the
/// maximizing L) was tried and rejected -- not for a fundamental reason, but
/// because `palette`'s `into_color()` clamps internally (see
/// `color::in_gamut`'s doc comment), so a boundary check built on it is a
/// tautology that always reports "in gamut." That's fixable with
/// `into_color_unclamped()` (see `color::gamut_map_oklch`, which needed the
/// same fix for actual gamut mapping), but piecewise-linear interpolation
/// between 6 known-good points is simpler, cheaper, and plenty accurate for
/// a lightness *curve bend*, not a precision gamut map -- so it stayed.
pub fn peak_lightness_for_hue(hue: f32) -> f32 {
    let mut anchors: Vec<(f32, f32)> = HueSlot::ALL
        .iter()
        .map(|s| {
            let lch = s.anchor_lch();
            (lch[2], lch[0])
        })
        .collect();
    anchors.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    let n = anchors.len();
    for i in 0..n {
        let (h0, l0) = anchors[i];
        let (h1, l1) = anchors[(i + 1) % n];
        let span = if h1 > h0 { h1 - h0 } else { h1 + 360.0 - h0 };
        let rel = if hue >= h0 {
            hue - h0
        } else {
            hue + 360.0 - h0
        };
        if rel <= span {
            let t = (rel / span).clamp(0.0, 1.0);
            return l0 + (l1 - l0) * t;
        }
    }
    anchors[0].1
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

/// Floor confidence multiplier for `fallback_rotate`'s chroma damping --
/// the worst case, a rotation with no real basis at all (`most_vivid` found
/// no qualifying cluster) or one dragged from the opposite side of the hue
/// wheel. Never damps to zero: even the least-confident fallback should
/// still register as *some* color, not vanish into gray.
const FALLBACK_MIN_CONFIDENCE: f32 = 0.5;

/// How much to trust a fallback rotation, based on how far the borrowed
/// cluster's actual hue sits from `target_hue`. A cluster just barely
/// outside `tolerance` (i.e. `match_hue` almost accepted it) is close to a
/// real match, so it keeps full confidence; a cluster from clear across the
/// hue wheel (180 degrees off) is close to a coin flip, so it's damped to
/// `FALLBACK_MIN_CONFIDENCE`. Replaces a flat, unexplained `* 0.8` constant
/// that damped every fallback the same amount regardless of how much of a
/// stretch the rotation actually was.
fn fallback_confidence(distance: f32, tolerance: f32) -> f32 {
    let worst = (180.0 - tolerance).max(1.0);
    let t = ((distance - tolerance) / worst).clamp(0.0, 1.0);
    1.0 - t * (1.0 - FALLBACK_MIN_CONFIDENCE)
}

/// When the image has no real presence near `target_hue`, borrow the most
/// vivid qualifying cluster (see `most_vivid`) and rotate it onto the
/// target hue, keeping its lightness (clamped to a workable range) and
/// damping its chroma by `fallback_confidence` to signal lower confidence
/// than a genuine match.
pub fn fallback_rotate(
    clusters: &[Cluster],
    target_hue: f32,
    stats: &ImageStats,
    tolerance: f32,
    min_weight: f32,
) -> [f32; 3] {
    let vivid = most_vivid(clusters, min_weight);

    let (l, c, confidence) = match vivid {
        Some(cluster) => {
            let lch = oklab_to_oklch(cluster.oklab);
            let distance = circular_diff(target_hue, lch[2]).abs();
            (
                lch[0].clamp(0.35, 0.75),
                lch[1].max(0.03),
                fallback_confidence(distance, tolerance),
            )
        }
        // No qualifying cluster at all -- this isn't rotating a real color,
        // it's guessing from whole-image stats, so it gets the floor
        // confidence outright rather than a computed distance.
        None => (
            stats.mean_l.clamp(0.35, 0.75),
            stats.mean_c.max(0.05),
            FALLBACK_MIN_CONFIDENCE,
        ),
    };
    [
        l,
        (c.min(stats.chromatic_mean_c.max(0.05) * 1.5)) * confidence,
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
        .unwrap_or_else(|| fallback_rotate(clusters, target, stats, tolerance, min_weight))
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

const MID_STEP: f32 = 500.0;

fn lightness_for_step(step: u16) -> f32 {
    let t = step as f32 / 950.0;
    0.95 - t * 0.80
}

/// Same shared curve as `lightness_for_step`, but bent per-hue so the ramp's
/// step-500 lightness moves toward wherever *this* hue can actually reach
/// its highest chroma (`peak_lightness_for_hue`) rather than every hue being
/// forced through the same generic 0.95->0.15 line. Pure yellow peaks near
/// L≈0.97; forcing it through the shared curve's L≈0.53 at step 500 is
/// exactly why mid-ramp yellow reads as olive/brown instead of vivid --
/// see "Known rough edges" in CLAUDE.md. `bend` is 0 (shared curve exactly,
/// old behavior) to 1 (step 500 lands exactly on the hue's peak).
///
/// Implementation: reparametrize `t` through a two-segment piecewise-linear
/// warp anchored at (t_for_step_500, t_for_blended_target), then evaluate
/// the original linear curve at the warped `t`. This guarantees the true
/// curve endpoints (t=0 -> L=0.95, t=1 -> L=0.15, i.e. exactly step 950)
/// stay fixed and the result stays monotonic, unlike fitting a quadratic
/// through three points which can overshoot the [0.15, 0.95] range or
/// wobble non-monotonically when the peak is close to an endpoint (e.g.
/// yellow). Step 50 (t≈0.053, not quite t=0) still shifts with `bend` --
/// for a hue whose peak sits far from the generic step-500 lightness, the
/// whole upper half of the ramp stretches toward it, not just step 500.
fn lightness_for_step_bent(step: u16, hue: f32, bend: f32) -> f32 {
    let t = step as f32 / 950.0;
    if bend <= 0.0 {
        return lightness_for_step(step);
    }
    let anchor_t = MID_STEP / 950.0;
    let generic_mid = lightness_for_step(MID_STEP as u16);
    let peak_l = peak_lightness_for_hue(hue);
    let target_mid = (generic_mid + (peak_l - generic_mid) * bend).clamp(0.15, 0.95);
    let t_mid = ((0.95 - target_mid) / 0.80).clamp(0.0, 1.0);
    let t_warped = if t <= anchor_t {
        t / anchor_t * t_mid
    } else {
        t_mid + (t - anchor_t) / (1.0 - anchor_t) * (1.0 - t_mid)
    };
    0.95 - t_warped * 0.80
}

/// Damp chroma toward the extremes of the lightness ramp so very light/dark
/// steps don't rely on out-of-gamut chroma that gets naively clipped.
fn chroma_taper(l: f32) -> f32 {
    let d = (l - 0.5).abs() * 2.0;
    (1.0 - d * 0.85).clamp(0.15, 1.0)
}

pub fn generate_ramp(anchor_lch: [f32; 3], chroma_cap: f32, hue_lightness_bend: f32) -> Ramp {
    let hue = anchor_lch[2];
    let base_chroma = anchor_lch[1].min(chroma_cap);
    RAMP_STEPS
        .iter()
        .map(|&step| {
            let l = lightness_for_step_bent(step, hue, hue_lightness_bend);
            // Taper against the step's *position* in the ramp (the
            // pre-bend generic curve), not its final bent lightness --
            // otherwise bending a hue's mid-ramp lightness away from 0.5
            // (e.g. yellow's step 500 moving to L≈0.84) gets read by the
            // taper as "near an extreme" and crushes exactly the chroma
            // the bend was meant to preserve.
            let c = base_chroma * chroma_taper(lightness_for_step(step));
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
    /// Prevalence-based (`pick_accent`): the single highest-weight cluster,
    /// exactly what matugen would extract. Kept for anyone who wants literal
    /// matugen parity, but this is a poor "accent" for a mostly-dark/muted
    /// wallpaper with a small vivid highlight -- the most common color there
    /// is just dark, not a color at all. `highlight` is the useful default.
    pub accent: Ramp,
    /// Vividness-based (`pick_vivid_accent`): what `primary` maps to by
    /// default, and what a "this wallpaper's accent color" UI element
    /// should reach for.
    pub highlight: Ramp,
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
            "highlight" => Some(&self.highlight),
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
    pub hue_lightness_bend: f32,
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
            // Nonzero by default (unlike neutral_accent_influence): this is
            // a correctness fix for the shared-lightness-curve rough edge,
            // not a stylistic judgment call. 0.7 rather than 1.0 so mid-ramp
            // still tracks the shared curve somewhat -- landing exactly on
            // each hue's peak-chroma lightness makes ramps across different
            // hues span visibly different lightness ranges, which looks
            // inconsistent for a themed ANSI16 set.
            hue_lightness_bend: 0.7,
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
        ramps.insert(
            slot.name(),
            generate_ramp(anchor, chroma_cap, params.hue_lightness_bend),
        );
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
    let neutral = generate_ramp(
        [0.5, neutral_chroma, neutral_hue],
        neutral_chroma,
        params.hue_lightness_bend,
    );

    let accent_anchor = pick_accent(clusters);
    let accent = generate_ramp(
        accent_anchor,
        chroma_cap.max(accent_anchor[1]),
        params.hue_lightness_bend,
    );
    let highlight = generate_ramp(
        vivid_accent,
        chroma_cap.max(vivid_accent[1]),
        params.hue_lightness_bend,
    );

    Primitives {
        red: ramps.remove("red").unwrap(),
        yellow: ramps.remove("yellow").unwrap(),
        green: ramps.remove("green").unwrap(),
        cyan: ramps.remove("cyan").unwrap(),
        blue: ramps.remove("blue").unwrap(),
        magenta: ramps.remove("magenta").unwrap(),
        neutral,
        accent,
        highlight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ramp_is_monotonically_lighter_toward_low_steps() {
        let ramp = generate_ramp([0.5, 0.15, 30.0], 0.2, 0.7);
        let l50 = ramp[&50].oklch[0];
        let l950 = ramp[&950].oklch[0];
        assert!(l50 > l950);
    }

    #[test]
    fn ramp_chroma_never_exceeds_cap() {
        let ramp = generate_ramp([0.5, 0.5, 30.0], 0.2, 0.7);
        for swatch in ramp.values() {
            assert!(swatch.oklch[1] <= 0.2 + 1e-4);
        }
    }

    #[test]
    fn ramp_is_monotonic_across_all_steps_when_bent() {
        // The piecewise-linear t-warp must stay monotonic end-to-end, not
        // just at the endpoints, for every bend strength -- otherwise a
        // "lighter" step could render darker than a step above it.
        for bend in [0.0, 0.3, 0.7, 1.0] {
            let ramp = generate_ramp([0.5, 0.15, 95.0], 0.2, bend);
            let mut prev_l = f32::INFINITY;
            for &step in RAMP_STEPS.iter() {
                let l = ramp[&step].oklch[0];
                assert!(l <= prev_l, "bend={bend} step={step} l={l} prev={prev_l}");
                prev_l = l;
            }
        }
    }

    #[test]
    fn bent_ramp_moves_step_500_toward_hue_peak_lightness() {
        // Yellow's peak-chroma lightness is much higher than the generic
        // curve's step-500 value (~0.53) -- bending should visibly move
        // step 500 toward that peak, which is the actual fix for the
        // "mid-ramp yellow reads as olive/brown" rough edge.
        let yellow_hue = HueSlot::Yellow.anchor_hue();
        let flat = generate_ramp([0.5, 0.1, yellow_hue], 0.2, 0.0);
        let bent = generate_ramp([0.5, 0.1, yellow_hue], 0.2, 0.7);
        assert!(bent[&500].oklch[0] > flat[&500].oklch[0] + 0.1);
        // The true curve floor (step 950, t=1) stays put regardless of bend.
        assert!((bent[&950].oklch[0] - flat[&950].oklch[0]).abs() < 1e-4);
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
        let lch = fallback_rotate(&clusters, 29.0, &stats, 30.0, 0.02);
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
        let lch = fallback_rotate(&clusters, 29.0, &stats, 30.0, 0.005);
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
        let lch = fallback_rotate(&clusters, 29.0, &stats, 30.0, 0.005);
        // L=0.3 clamped up to the 0.35 floor -- the dull cluster's L, not
        // the vivid-but-too-small cluster's L=0.5.
        assert!((lch[0] - 0.35).abs() < 1e-3);
    }

    #[test]
    fn fallback_rotate_damps_chroma_less_for_a_near_miss_than_a_far_miss() {
        // Both clusters are equally vivid and equally weighted; only their
        // distance from target_hue=29.0 differs. A cluster just past
        // tolerance=30 (hue 61, distance 32) is nearly a real match and
        // should keep most of its chroma; one from the opposite side of the
        // wheel (hue 209, distance 180) should be damped much harder.
        let near_miss = vec![Cluster {
            oklab: oklch_to_oklab([0.5, 0.2, 61.0]),
            weight: 0.5,
        }];
        let far_miss = vec![Cluster {
            oklab: oklch_to_oklab([0.5, 0.2, 209.0]),
            weight: 0.5,
        }];
        let stats = ImageStats {
            mean_l: 0.5,
            mean_c: 0.1,
            chromatic_mean_c: 0.2,
            is_dark: false,
        };
        let near = fallback_rotate(&near_miss, 29.0, &stats, 30.0, 0.005);
        let far = fallback_rotate(&far_miss, 29.0, &stats, 30.0, 0.005);
        assert!(near[1] > far[1]);
        // Far miss should land at (or very near) the floor confidence.
        let expected_far_c =
            0.2f32.min(stats.chromatic_mean_c.max(0.05) * 1.5) * FALLBACK_MIN_CONFIDENCE;
        assert!((far[1] - expected_far_c).abs() < 1e-3);
    }

    #[test]
    fn fallback_rotate_with_no_qualifying_cluster_gets_floor_confidence() {
        // No cluster clears min_weight -- most_vivid returns None, so this
        // is a pure stats-based guess and should get the same floor
        // confidence as the worst-case rotation distance, not a computed
        // in-between value.
        let clusters = vec![Cluster {
            oklab: oklch_to_oklab([0.5, 0.2, 61.0]),
            weight: 0.001,
        }];
        let stats = ImageStats {
            mean_l: 0.5,
            mean_c: 0.1,
            chromatic_mean_c: 0.2,
            is_dark: false,
        };
        let lch = fallback_rotate(&clusters, 29.0, &stats, 30.0, 0.005);
        let expected_c = stats
            .mean_c
            .max(0.05)
            .min(stats.chromatic_mean_c.max(0.05) * 1.5)
            * FALLBACK_MIN_CONFIDENCE;
        assert!((lch[1] - expected_c).abs() < 1e-4);
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
        // Taper is against ramp *position* (the pre-bend generic curve),
        // not the final bent lightness -- see generate_ramp's comment.
        let expected_c = params.neutral_tint_chroma * chroma_taper(lightness_for_step(500));
        assert!((n500.oklch[1] - expected_c).abs() < 1e-4);
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

    #[test]
    fn peak_lightness_for_hue_matches_known_anchors_exactly() {
        // Feeding a known anchor's own hue back in should return that
        // anchor's own L exactly (t=0 on the interpolation segment).
        for slot in HueSlot::ALL {
            let lch = slot.anchor_lch();
            assert!((peak_lightness_for_hue(lch[2]) - lch[0]).abs() < 1e-4);
        }
    }

    #[test]
    fn peak_lightness_for_hue_yellow_much_higher_than_blue() {
        let yellow_peak = peak_lightness_for_hue(HueSlot::Yellow.anchor_hue());
        let blue_peak = peak_lightness_for_hue(HueSlot::Blue.anchor_hue());
        assert!(yellow_peak > 0.9);
        assert!(blue_peak < 0.55);
    }
}
