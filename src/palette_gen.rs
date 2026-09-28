use crate::color::{
    circular_diff, lerp_hue, max_chroma, oklab_to_oklch, oklab_to_srgb_u8, oklch_to_oklab,
    pure_corner_oklch, relative_chroma, to_hex,
};
use crate::extract::{CHROMATIC_THRESHOLD, Cluster, ImageStats};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Swatch {
    pub hex: String,
    pub hex_stripped: String,
    pub rgb: [u8; 3],
    pub oklch: [f32; 3],
}

fn swatch_from_oklch(lch: [f32; 3]) -> Swatch {
    let lab = oklch_to_oklab(lch);
    let rgb = oklab_to_srgb_u8(lab);
    let hex = to_hex(rgb);
    let hex_stripped = hex.trim_start_matches('#').to_string();
    Swatch {
        hex,
        hex_stripped,
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

/// Hue of pure sRGB orange (#ff8000). Not an ANSI slot, but a boundary for
/// `neighbor_gaps`: the red->yellow gap (~81 deg, by far the widest) holds
/// a whole named color of its own, so a fraction of the full gap let red
/// shift squarely into orange -- on an amber wallpaper `red.500` came out
/// identical to `primary`, and `danger` with it. Measuring red's and
/// yellow's gaps to orange instead keeps red red. The other wide gaps
/// (cyan->blue, blue->magenta) get no such boundary on purpose: Oklch packs
/// azure/violet close to blue, and a blue leaning toward a sky between cyan
/// and blue is exactly the wallpaper character worth keeping.
fn orange_boundary_hue() -> f32 {
    pure_corner_oklch([255, 128, 0])[2]
}

/// Hue gaps from the slot anchor nearest `hue` to its two neighbors on the
/// wheel: `(toward lower hue, toward higher hue)`, in degrees. Neighbors
/// are the other `HueSlot` anchors plus `orange_boundary_hue`, all computed
/// (not hardcoded), same as `peak_lightness_for_hue`. The gaps are very
/// uneven -- yellow->green is ~33 deg, cyan->blue ~69 -- which is why
/// `clamp_hue_shift` limits shifts as a fraction of the gap rather than a
/// fixed number of degrees.
pub fn neighbor_gaps(hue: f32) -> (f32, f32) {
    let mut marks: Vec<(f32, bool)> = HueSlot::ALL
        .iter()
        .map(|s| (s.anchor_hue(), true))
        .chain(std::iter::once((orange_boundary_hue(), false)))
        .collect();
    marks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let n = marks.len();
    let i = (0..n)
        .filter(|&i| marks[i].1)
        .min_by(|&a, &b| {
            circular_diff(hue, marks[a].0)
                .abs()
                .partial_cmp(&circular_diff(hue, marks[b].0).abs())
                .unwrap()
        })
        .expect("slot anchors are non-empty");
    let below = circular_diff(marks[(i + n - 1) % n].0, marks[i].0).abs();
    let above = circular_diff(marks[i].0, marks[(i + 1) % n].0).abs();
    (below, above)
}

/// True if `hue` is at least as close to `target_hue` as to any `HueSlot`
/// anchor -- i.e. the color belongs to that slot's own hue family rather
/// than a neighbor's (an acid green 34 deg from yellow is still nearer
/// green's anchor, so it's green's, not yellow's).
fn in_family(target_hue: f32, hue: f32) -> bool {
    let own = circular_diff(target_hue, hue).abs();
    HueSlot::ALL
        .iter()
        .all(|s| own <= circular_diff(s.anchor_hue(), hue).abs() + 1e-3)
}

/// Move `anchor_hue` toward `matched_hue`, but by at most `limit` times the
/// gap to the neighbor on that side (see `neighbor_gaps`). Keeps
/// a matched slot inside its own hue family even when `hue_tolerance` is
/// wide enough to reach a neighbor's colors: `limit` 0 = exact anchor hue,
/// 1 = could reach the neighbor. Taking the cluster's hue outright
/// (the previous behavior) turned yellow green on a mostly-green wallpaper
/// at `hue_tolerance = 35`, and made cyan/blue identical when both matched
/// one sky cluster sitting between them -- see `HISTORY.md`.
pub fn clamp_hue_shift(anchor_hue: f32, matched_hue: f32, limit: f32) -> f32 {
    let (below, above) = neighbor_gaps(anchor_hue);
    let limit = limit.clamp(0.0, 1.0);
    let offset = circular_diff(anchor_hue, matched_hue).clamp(-below * limit, above * limit);
    (anchor_hue + offset).rem_euclid(360.0)
}

/// Most *vivid* (highest-chroma) weighted cluster within `tolerance` degrees
/// of `target_hue`, carrying at least `min_weight` share of the image. Skips
/// near-neutral clusters since a gray pixel's hue angle is noise, not signal.
///
/// Picks by chroma, not nearest angle: nearest-angle-wins let a weak, barely-
/// qualifying cluster (technically in tolerance but low chroma) outrank a
/// more vivid cluster just a few degrees farther off, producing a *more*
/// muted color than the (then vividness-picking) fallback would've given
/// for a non-match at all, so a "weak real match" could end up worse than
/// "no match." Widening `hue_tolerance` to fix muted colors made this worse
/// in practice, not better: it let more weak clusters qualify as "real"
/// matches, displacing the vividness-aware fallback path entirely. (Muted
/// matches are also lifted toward the primary's vibrancy now, see
/// `resolve_hue_slot`'s `vibrancy_coherence`.)
///
/// Own-family clusters (`in_family`) outrank neighbor-family ones regardless
/// of chroma; vividness only decides within each group. Otherwise a wide
/// tolerance lets a vivid neighbor color (acid green) beat a real but softer
/// in-family one (an actual yellow) for the yellow slot, and
/// `clamp_hue_shift` could only hide that mistake, not undo it.
pub fn match_hue(
    clusters: &[Cluster],
    target_hue: f32,
    tolerance: f32,
    min_weight: f32,
) -> Option<[f32; 3]> {
    let mut best: Option<(bool, [f32; 3])> = None;
    for c in clusters {
        if c.weight < min_weight {
            continue;
        }
        let lch = oklab_to_oklch(c.oklab);
        if lch[1] < 0.01 {
            continue;
        }
        let d = circular_diff(target_hue, lch[2]).abs();
        if d > tolerance {
            continue;
        }
        let family = in_family(target_hue, lch[2]);
        let better = match best {
            None => true,
            Some((best_family, b)) => (family, lch[1]) > (best_family, b[1]),
        };
        if better {
            best = Some((family, lch));
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
/// Backs `pick_vivid_accent`, and through it `primary_vibrancy`.
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

/// Floor for the vibrancy every chromatic ANSI ramp is generated at (see
/// `primary_vibrancy`). On a near-grayscale wallpaper the vivid accent's
/// relative chroma is close to 0, and copying that onto the hue slots would
/// collapse red/green/yellow/... into indistinguishable grays -- breaking
/// the one contract this tool exists for (red still reads as red). Same
/// spirit as a confidence floor: never let a hue slot vanish into gray.
pub const MIN_RELATIVE_CHROMA: f32 = 0.25;

/// Target vibrancy for the chromatic ramps: the vivid accent's (`primary`'s)
/// relative chroma (`color::relative_chroma` -- chroma as a fraction of what
/// its hue can reach at its lightness), floored at `MIN_RELATIVE_CHROMA`.
/// Relative, not absolute: an absolute chroma cap shared across hues made
/// red/blue/magenta read pastel next to a neon primary, since the same C
/// is a much smaller share of those hues' gamut -- see `HISTORY.md`.
pub fn primary_vibrancy(vivid_accent: [f32; 3]) -> f32 {
    relative_chroma(vivid_accent).max(MIN_RELATIVE_CHROMA)
}

/// Where a chromatic hue slot's ramp comes from: which hue it's generated
/// at, and how vivid it is as a fraction of that hue's gamut at each step
/// (see `generate_ramp`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SlotAnchor {
    pub hue: f32,
    pub relative_chroma: f32,
    /// True if a real wallpaper cluster was matched (`match_hue`), false if
    /// the slot fell back to its pure anchor hue.
    pub matched: bool,
}

/// Resolve a hue slot against the wallpaper:
///
/// - **Matched** (`match_hue` found a cluster): the anchor hue shifted toward
///   the cluster's, at most `hue_shift_limit` of the way to the neighboring
///   anchor (`clamp_hue_shift`), and a
///   vibrancy blended from the cluster's own relative chroma toward
///   `primary_r` by `vibrancy_coherence` -- 1 = every slot as vivid as the
///   primary, 0 = the cluster's own saturation kept as-is (a muted blue in
///   an otherwise vivid wallpaper stays muted).
/// - **Fallback** (no cluster near this hue): the slot's exact anchor hue at
///   `primary_r * fallback_vibrancy`, floored at `MIN_RELATIVE_CHROMA`. This
///   replaced rotating the most vivid cluster onto the target hue and
///   damping its chroma by rotation distance (`fallback_confidence`): that
///   damping was tied to how far the borrowed cluster sat on the hue wheel
///   and made a fallback red read pastel next to a neon primary. A flat,
///   user-tunable scale keeps fallbacks proportional to the primary -- a
///   hue the wallpaper doesn't have at all shouldn't be the loudest color
///   in the palette (a pure-magenta slot at full vibrancy next to an orange
///   primary was, see `HISTORY.md`), but shouldn't collapse to pastel either.
pub fn resolve_hue_slot(
    slot: HueSlot,
    clusters: &[Cluster],
    primary_r: f32,
    params: &GenParams,
) -> SlotAnchor {
    let target = slot.anchor_hue();
    match match_hue(
        clusters,
        target,
        params.hue_tolerance,
        params.min_cluster_weight,
    ) {
        Some(lch) => {
            let own = relative_chroma(lch);
            let t = params.vibrancy_coherence.clamp(0.0, 1.0);
            SlotAnchor {
                hue: clamp_hue_shift(target, lch[2], params.hue_shift_limit),
                relative_chroma: own + (primary_r - own) * t,
                matched: true,
            }
        }
        None => SlotAnchor {
            hue: target,
            relative_chroma: (primary_r * params.fallback_vibrancy.clamp(0.0, 1.0))
                .max(MIN_RELATIVE_CHROMA),
            matched: false,
        },
    }
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
///
/// `min_mid` floors step 500's lightness after the bend. The bend chases
/// peak *chroma*, which only helps readability for hues that peak light
/// (yellow): pure blue peaks at L≈0.45, below the generic 0.53, so the bend
/// pushed blue.500 down to L≈0.48 -- ~2.8:1 on a dark background. The floor
/// lifts it without moving the blue anchor toward cyan: at equal L, hue
/// barely changes contrast, so lightness is the lever, not hue (see
/// `HISTORY.md`). Same warp, so the ramp stays monotonic with fixed ends.
fn lightness_for_step_bent(step: u16, hue: f32, bend: f32, min_mid: f32) -> f32 {
    let t = step as f32 / 950.0;
    let generic_mid = lightness_for_step(MID_STEP as u16);
    if bend <= 0.0 && min_mid <= generic_mid {
        return lightness_for_step(step);
    }
    let anchor_t = MID_STEP / 950.0;
    let peak_l = peak_lightness_for_hue(hue);
    let target_mid = (generic_mid + (peak_l - generic_mid) * bend)
        .max(min_mid)
        .clamp(0.15, 0.95);
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

/// Chromatic ramp at `hue`, where every step's chroma is the same fraction
/// (`relative_chroma`, 0..1) of what that hue can actually reach at the
/// step's lightness (`color::max_chroma`), shaped by `chroma_taper` toward
/// the ramp's extremes -- normalized so step 500 carries `relative_chroma`
/// exactly (the taper would otherwise shave ~5% off there, making the
/// primary's own 500 read slightly less vivid than the cluster it came from). An absolute chroma shared across the ramp (the
/// previous behavior) is a different share of the gamut at every step and
/// every hue -- a C that's neon at one hue is pastel at another -- which is
/// what made ANSI slots read washed-out next to the primary. Also makes the
/// taper a purely aesthetic shaping now, not a gamut-clipping guard: the
/// relative target is in gamut by construction.
///
/// `min_mid_lightness` floors step 500 (see `lightness_for_step_bent`).
pub fn generate_ramp(
    hue: f32,
    relative_chroma: f32,
    hue_lightness_bend: f32,
    min_mid_lightness: f32,
) -> Ramp {
    let r = relative_chroma.clamp(0.0, 1.0);
    let taper_mid = chroma_taper(lightness_for_step(MID_STEP as u16));
    RAMP_STEPS
        .iter()
        .map(|&step| {
            let l = lightness_for_step_bent(step, hue, hue_lightness_bend, min_mid_lightness);
            // Taper against the step's *position* in the ramp -- see
            // `generate_absolute_ramp_inner`.
            let taper = chroma_taper(lightness_for_step(step)) / taper_mid;
            let c = (r * taper).min(1.0) * max_chroma(l, hue);
            (step, swatch_from_oklch([l, c, hue]))
        })
        .collect()
}

/// Absolute-chroma ramp: `anchor_lch`'s chroma (capped at `chroma_cap`)
/// held across every step, tapered toward the extremes. Only `accent` uses
/// this now -- it's the literal matugen-parity ramp, so it keeps the
/// original generation exactly rather than the relative model the
/// chromatic ramps moved to.
pub fn generate_absolute_ramp(
    anchor_lch: [f32; 3],
    chroma_cap: f32,
    hue_lightness_bend: f32,
) -> Ramp {
    generate_absolute_ramp_inner(anchor_lch, chroma_cap, hue_lightness_bend, true)
}

/// Like `generate_absolute_ramp` but skips both `chroma_taper` and the
/// per-hue lightness bend.
///
/// No bend: `hue_lightness_bend` moves step 500 toward the lightness where
/// the ramp's *hue* peaks in chroma -- a fix for vivid ramps (yellow reading
/// olive), meaningless for a near-gray one whose chroma is tiny by design.
/// Applied here anyway, it just lifted the whole middle of the ramp on
/// green/amber-tinted wallpapers (their peaks sit at L~0.8-0.9): step 600
/// (`ansi_color8`) landed at L~0.60-0.66 instead of ~0.45, reading as a mid
/// gray rather than a bright black. Neutral steps now follow the plain
/// lightness curve, so they mean the same thing on every wallpaper.
///
/// No taper: `chroma_taper` exists to
/// keep vivid *chromatic* ramps (red/yellow/green/...) from relying on
/// out-of-gamut chroma near the ramp's extremes -- but the neutral ramp's
/// chroma is tiny by design (`neutral_tint_chroma` default 0.015, an order
/// of magnitude under any real hue's gamut boundary ~0.2+), so it was never
/// at gamut-clipping risk in the first place. Applying the same extreme-step
/// crush there just defeats `neutral_accent_influence` at bg/color0 --
/// `auto:bg`/`ansi_color0` resolve to the ramp's darkest step (950 for dark
/// wallpapers), exactly where the taper bites hardest, while `ansi_color7`/
/// `ansi_color8` (mid-ramp steps 300/600, taper barely applies) visibly show
/// the tint -- same knob, wildly different visible effect depending on which
/// step a role happens to sit at. `gamut_map_oklch` (applied downstream in
/// `swatch_from_oklch`) is still the real safety net for genuine
/// out-of-gamut colors, so removing the extra taper here doesn't reopen the
/// clipping problem the taper was originally added for.
pub fn generate_neutral_ramp(anchor_lch: [f32; 3], chroma_cap: f32) -> Ramp {
    generate_absolute_ramp_inner(anchor_lch, chroma_cap, 0.0, false)
}

fn generate_absolute_ramp_inner(
    anchor_lch: [f32; 3],
    chroma_cap: f32,
    hue_lightness_bend: f32,
    taper: bool,
) -> Ramp {
    let hue = anchor_lch[2];
    let base_chroma = anchor_lch[1].min(chroma_cap);
    RAMP_STEPS
        .iter()
        .map(|&step| {
            // No lightness floor: `accent` is literal matugen parity and
            // `neutral` must mean the same L on every wallpaper.
            let l = lightness_for_step_bent(step, hue, hue_lightness_bend, 0.0);
            // Taper against the step's *position* in the ramp (the
            // pre-bend generic curve), not its final bent lightness --
            // otherwise bending a hue's mid-ramp lightness away from 0.5
            // (e.g. yellow's step 500 moving to L≈0.84) gets read by the
            // taper as "near an extreme" and crushes exactly the chroma
            // the bend was meant to preserve.
            let c = if taper {
                base_chroma * chroma_taper(lightness_for_step(step))
            } else {
                base_chroma
            };
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
    pub min_mid_lightness: f32,
    pub vibrancy_coherence: f32,
    pub fallback_vibrancy: f32,
    pub hue_shift_limit: f32,
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
            // ~5:1 against a dark background, about where red.500 already
            // sat -- in practice it only lifts blue (bent to L≈0.48, ~2.8:1)
            // and, marginally, red. Chromatic ramps and highlight only.
            min_mid_lightness: 0.62,
            // Mostly, not fully, toward the primary: a slot that genuinely
            // matched a muted cluster (vivid greens/reds but soft blues)
            // gets lifted so it doesn't read pastel next to its neon
            // siblings, but keeps a hint of the wallpaper's own softness.
            // Fallback slots always use the primary's vibrancy outright.
            vibrancy_coherence: 0.7,
            // Hues absent from the wallpaper at 3/4 of the primary's
            // vibrancy: full vibrancy made them the loudest colors in the
            // palette (pure magenta next to an orange primary), while the
            // matched slots they sit beside get pulled toward the primary
            // only partway (vibrancy_coherence) from usually-lower values.
            fallback_vibrancy: 0.75,
            // Under 0.5 so two neighboring slots can never cross each other,
            // with room to spare: at 0.3 yellow (the tightest gap, ~33 deg to
            // green) moves at most ~10 deg, while blue can still lean ~21
            // deg toward a sky that sits between cyan and blue.
            hue_shift_limit: 0.3,
        }
    }
}

pub fn build_primitives(
    clusters: &[Cluster],
    stats: &ImageStats,
    params: &GenParams,
) -> Primitives {
    // Every chromatic ramp is generated at a vibrancy relative to its own
    // hue's gamut, anchored to the primary's (see `primary_vibrancy`).
    let vivid_accent = pick_vivid_accent(clusters, params.min_cluster_weight);
    let primary_r = primary_vibrancy(vivid_accent);

    let mut ramps: BTreeMap<&'static str, Ramp> = BTreeMap::new();
    for slot in HueSlot::ALL {
        let anchor = resolve_hue_slot(slot, clusters, primary_r, params);
        ramps.insert(
            slot.name(),
            generate_ramp(
                anchor.hue,
                anchor.relative_chroma,
                params.hue_lightness_bend,
                params.min_mid_lightness,
            ),
        );
    }

    // Absolute cap for the neutral tint and the matugen-parity `accent`
    // ramp only -- the chromatic ramps above are relative now.
    // chromatic_mean_c (not mean_c): a mostly-dark/desaturated image
    // shouldn't have its accent vividness crushed by pixels that are
    // achromatic in the first place -- see ImageStats::chromatic_mean_c.
    let chroma_cap = stats.chromatic_mean_c.max(0.02) * params.chroma_clamp_factor;

    // bg/fg tint: blend from the flat average-hue/fixed-chroma baseline
    // (influence=0, original behavior) toward the wallpaper's vivid accent
    // (influence=1) -- see pick_vivid_accent's doc comment for why "vivid"
    // and not "prevalent" is the right notion of accent here.
    let base_hue = weighted_mean_hue(clusters);
    let influence = params.neutral_accent_influence.clamp(0.0, 1.0);
    let neutral_hue = lerp_hue(base_hue, vivid_accent[2], influence);
    let vivid_chroma = vivid_accent[1].min(chroma_cap);
    let neutral_chroma = params.neutral_tint_chroma
        + (vivid_chroma - params.neutral_tint_chroma).max(0.0) * influence;
    let neutral = generate_neutral_ramp([0.5, neutral_chroma, neutral_hue], neutral_chroma);

    let accent_anchor = pick_accent(clusters);
    let accent = generate_absolute_ramp(
        accent_anchor,
        chroma_cap.max(accent_anchor[1]),
        params.hue_lightness_bend,
    );
    // Same relative model as the hue slots so `primary` and the ANSI 500s
    // share one vibrancy scale -- but the accent's *own* relative chroma,
    // not `primary_r`'s floored one: on a genuinely gray wallpaper the
    // primary should honestly read gray, the floor is only there to keep
    // the ANSI hues telling apart.
    let highlight = generate_ramp(
        vivid_accent[2],
        relative_chroma(vivid_accent),
        params.hue_lightness_bend,
        params.min_mid_lightness,
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
        let ramp = generate_ramp(30.0, 0.8, 0.7, 0.0);
        let l50 = ramp[&50].oklch[0];
        let l950 = ramp[&950].oklch[0];
        assert!(l50 > l950);
    }

    #[test]
    fn ramp_holds_the_same_gamut_fraction_across_hues() {
        // The whole point of the relative model: a given vibrancy is the
        // same share of the gamut at every hue, so red.500 is exactly as
        // "neon for a red" as cyan.500 is for a cyan -- not the same
        // absolute C, which would be pastel for one and loud for the other.
        for slot in HueSlot::ALL {
            let ramp = generate_ramp(slot.anchor_hue(), 0.8, 0.7, 0.0);
            let r = relative_chroma(ramp[&500].oklch);
            assert!((r - 0.8).abs() < 0.01, "{} r={r}", slot.name());
        }
    }

    #[test]
    fn ramp_stays_in_gamut_at_full_vibrancy() {
        // r=1 targets max_chroma exactly at every step, so gamut mapping in
        // swatch_from_oklch should be a no-op (chroma not reduced).
        let ramp = generate_ramp(HueSlot::Magenta.anchor_hue(), 1.0, 0.7, 0.0);
        for (&step, swatch) in &ramp {
            let max = max_chroma(swatch.oklch[0], swatch.oklch[2]);
            assert!(swatch.oklch[1] <= max + 1e-4, "step={step}");
        }
    }

    #[test]
    fn ramp_is_monotonic_across_all_steps_when_bent() {
        // The piecewise-linear t-warp must stay monotonic end-to-end, not
        // just at the endpoints, for every bend strength -- otherwise a
        // "lighter" step could render darker than a step above it.
        for bend in [0.0, 0.3, 0.7, 1.0] {
            let ramp = generate_ramp(95.0, 0.8, bend, 0.0);
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
        let flat = generate_ramp(yellow_hue, 0.5, 0.0, 0.0);
        let bent = generate_ramp(yellow_hue, 0.5, 0.7, 0.0);
        assert!(bent[&500].oklch[0] > flat[&500].oklch[0] + 0.1);
        // The true curve floor (step 950, t=1) stays put regardless of bend.
        assert!((bent[&950].oklch[0] - flat[&950].oklch[0]).abs() < 1e-4);
    }

    #[test]
    fn lightness_floor_lifts_blue_500_off_the_bend() {
        // Regression: blue peaks at L≈0.45, so the bend alone dragged
        // blue.500 below the generic curve to L≈0.48 (~2.8:1 on dark bg).
        let blue = HueSlot::Blue.anchor_hue();
        let bent = generate_ramp(blue, 0.9, 0.7, 0.0);
        assert!(bent[&500].oklch[0] < 0.5);
        let floored = generate_ramp(blue, 0.9, 0.7, 0.62);
        assert!((floored[&500].oklch[0] - 0.62).abs() < 1e-3);
        // Hue stays blue: the floor is a lightness fix, not a hue shift.
        assert!(circular_diff(floored[&500].oklch[2], blue).abs() < 1.0);
        // Ramp ends don't move.
        assert!((floored[&950].oklch[0] - bent[&950].oklch[0]).abs() < 1e-4);
    }

    #[test]
    fn lightness_floor_leaves_hues_already_above_it_alone() {
        // Yellow/green/cyan bend well above any sane floor.
        for slot in [HueSlot::Yellow, HueSlot::Green, HueSlot::Cyan] {
            let hue = slot.anchor_hue();
            let plain = generate_ramp(hue, 0.8, 0.7, 0.0);
            let floored = generate_ramp(hue, 0.8, 0.7, 0.62);
            for &step in RAMP_STEPS.iter() {
                assert_eq!(
                    plain[&step].hex,
                    floored[&step].hex,
                    "{} {step}",
                    slot.name()
                );
            }
        }
    }

    #[test]
    fn lightness_floor_keeps_ramp_monotonic() {
        for slot in HueSlot::ALL {
            for floor in [0.62, 0.7, 0.9] {
                let ramp = generate_ramp(slot.anchor_hue(), 0.8, 0.7, floor);
                let mut prev_l = f32::INFINITY;
                for &step in RAMP_STEPS.iter() {
                    let l = ramp[&step].oklch[0];
                    assert!(l <= prev_l, "{} floor={floor} step={step}", slot.name());
                    prev_l = l;
                }
                assert!(ramp[&500].oklch[0] >= floor - 1e-3, "{}", slot.name());
            }
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
    fn match_hue_prefers_vivid_cluster_over_nearer_weak_one() {
        // Both clusters are within tolerance of target_hue=29.0: one sits
        // closer in angle but is nearly gray (C=0.02), the other is farther
        // but much more vivid (C=0.2). Picking by nearest angle would return
        // the weak one and produce a muted color despite a vivid candidate
        // being available -- match_hue should prefer the vivid one instead.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.02, 32.0]),
                weight: 0.5,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.2, 50.0]),
                weight: 0.5,
            },
        ];
        let found = match_hue(&clusters, 29.0, 30.0, 0.01).unwrap();
        assert!((found[2] - 50.0).abs() < 1.0);
        assert!((found[1] - 0.2).abs() < 1e-3);
    }

    #[test]
    fn match_hue_prefers_own_family_over_more_vivid_neighbor() {
        // A soft real yellow vs. a vivid acid green that's still within a
        // wide tolerance of the yellow anchor -- the yellow slot should
        // take its own family's color, however much more vivid the green.
        let yellow = HueSlot::Yellow.anchor_hue();
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.8, 0.08, yellow + 3.0]),
                weight: 0.2,
            },
            Cluster {
                oklab: oklch_to_oklab([0.8, 0.19, 144.0]),
                weight: 0.2,
            },
        ];
        let found = match_hue(&clusters, yellow, 35.0, 0.01).unwrap();
        assert!((found[2] - (yellow + 3.0)).abs() < 0.5);
        // With no in-family candidate, the neighbor's color still matches.
        let found = match_hue(&clusters[1..], yellow, 35.0, 0.01).unwrap();
        assert!((found[2] - 144.0).abs() < 0.5);
    }

    #[test]
    fn neighbor_gaps_match_adjacent_anchors() {
        let (below, above) = neighbor_gaps(HueSlot::Yellow.anchor_hue());
        let red = HueSlot::Red.anchor_hue();
        let yellow = HueSlot::Yellow.anchor_hue();
        let green = HueSlot::Green.anchor_hue();
        let orange = orange_boundary_hue();
        assert!(red < orange && orange < yellow);
        // Red and yellow measure to the orange boundary, not each other.
        assert!((below - (yellow - orange)).abs() < 1e-3);
        assert!((above - (green - yellow)).abs() < 1e-3);
        let (_, red_above) = neighbor_gaps(red);
        assert!((red_above - (orange - red)).abs() < 1e-3);
        // Red's lower neighbor is magenta, across the 0/360 wrap.
        let (below, _) = neighbor_gaps(red);
        let magenta = HueSlot::Magenta.anchor_hue();
        assert!((below - (red + 360.0 - magenta)).abs() < 1e-3);
    }

    #[test]
    fn clamp_hue_shift_limits_by_the_gap_on_the_shifted_side() {
        let yellow = HueSlot::Yellow.anchor_hue();
        let (below, above) = neighbor_gaps(yellow);
        // Toward green (tight gap): clamped to 0.3 * ~33 deg.
        let h = clamp_hue_shift(yellow, yellow + 34.0, 0.3);
        assert!((h - (yellow + 0.3 * above)).abs() < 1e-3);
        // Toward orange (wider gap): a 10 deg shift is under 0.3 * ~55, kept.
        let h = clamp_hue_shift(yellow, yellow - 10.0, 0.3);
        assert!((h - (yellow - 10.0)).abs() < 1e-3);
        let h = clamp_hue_shift(yellow, yellow - 60.0, 0.3);
        assert!((h - (yellow - 0.3 * below)).abs() < 1e-3);
        // limit 0 pins the anchor hue exactly.
        assert!((clamp_hue_shift(yellow, yellow + 20.0, 0.0) - yellow).abs() < 1e-3);
    }

    #[test]
    fn matched_slots_never_cross_neighbors_below_half_limit() {
        // Worst case: every slot matched a cluster sitting right on its
        // neighbor's anchor, in both directions. Below limit 0.5 the six
        // hues must keep their order around the wheel.
        for limit in [0.3, 0.49] {
            for toward_higher in [false, true] {
                let mut hues: Vec<(f32, f32)> = HueSlot::ALL
                    .iter()
                    .map(|s| {
                        let a = s.anchor_hue();
                        let (below, above) = neighbor_gaps(a);
                        let target = if toward_higher { a + above } else { a - below };
                        (a, clamp_hue_shift(a, target.rem_euclid(360.0), limit))
                    })
                    .collect();
                hues.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
                for i in 0..hues.len() {
                    let (_, h0) = hues[i];
                    let (_, h1) = hues[(i + 1) % hues.len()];
                    assert!(
                        circular_diff(h0, h1) > 0.0,
                        "limit={limit} toward_higher={toward_higher} {h0} vs {h1}"
                    );
                }
            }
        }
    }

    #[test]
    fn red_stays_red_when_the_primary_is_orange() {
        // The pixel-art-castle regression: an amber/orange primary matched
        // by the red slot at hue_tolerance = 30 came out identical to the
        // primary. Red must stay within the limit of its gap to orange.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.25, 0.03, 80.0]),
                weight: 0.8,
            },
            Cluster {
                oklab: oklch_to_oklab([0.65, 0.17, 52.0]),
                weight: 0.2,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.3,
            mean_c: 0.05,
            chromatic_mean_c: 0.06,
            is_dark: true,
        };
        let params = GenParams {
            hue_tolerance: 30.0,
            ..GenParams::default()
        };
        let p = build_primitives(&clusters, &stats, &params);
        let red = HueSlot::Red.anchor_hue();
        let (_, above) = neighbor_gaps(red);
        let shift = circular_diff(red, p.red[&500].oklch[2]);
        assert!(
            shift <= params.hue_shift_limit * above + 0.5,
            "shift={shift}"
        );
        assert_ne!(p.red[&500].hex, p.highlight[&500].hex);
    }

    #[test]
    fn wide_tolerance_keeps_yellow_yellow_on_a_green_wallpaper() {
        // The Hollow Knight regression at hue_tolerance = 35: acid greens
        // ~34 deg from the yellow anchor used to be taken outright.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.22, 0.035, 215.0]),
                weight: 0.7,
            },
            Cluster {
                oklab: oklch_to_oklab([0.81, 0.186, 144.0]),
                weight: 0.3,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.34,
            mean_c: 0.065,
            chromatic_mean_c: 0.065,
            is_dark: true,
        };
        let params = GenParams {
            hue_tolerance: 35.0,
            ..GenParams::default()
        };
        let p = build_primitives(&clusters, &stats, &params);
        let yellow = HueSlot::Yellow.anchor_hue();
        let (_, above) = neighbor_gaps(yellow);
        let shift = circular_diff(yellow, p.yellow[&500].oklch[2]);
        assert!(
            shift <= params.hue_shift_limit * above + 0.5,
            "shift={shift}"
        );
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
    fn unmatched_slot_lands_on_anchor_hue_at_primary_vibrancy() {
        // Nothing near red: the slot takes red's exact anchor hue and the
        // primary's vibrancy scaled by fallback_vibrancy -- a flat scale,
        // not a damping by how far the nearest cluster sits on the wheel.
        let clusters = vec![Cluster {
            oklab: oklch_to_oklab([0.6, 0.15, 150.0]),
            weight: 0.9,
        }];
        let params = GenParams {
            fallback_vibrancy: 0.5,
            ..GenParams::default()
        };
        let slot = resolve_hue_slot(HueSlot::Red, &clusters, 0.8, &params);
        assert!(!slot.matched);
        assert!((slot.hue - HueSlot::Red.anchor_hue()).abs() < 1e-3);
        assert!((slot.relative_chroma - 0.4).abs() < 1e-6);
        // Scaling never pushes a fallback below the distinguishability floor.
        let params = GenParams {
            fallback_vibrancy: 0.1,
            ..GenParams::default()
        };
        let slot = resolve_hue_slot(HueSlot::Red, &clusters, 0.8, &params);
        assert!((slot.relative_chroma - MIN_RELATIVE_CHROMA).abs() < 1e-6);
    }

    #[test]
    fn matched_slot_blends_own_vibrancy_toward_primary_by_coherence() {
        // A real but muted red cluster: coherence 0 keeps its own
        // vibrancy, 1 lifts it all the way to the primary's, 0.5 halfway.
        let muted = [0.55, 0.05, 30.0];
        let clusters = vec![Cluster {
            oklab: oklch_to_oklab(muted),
            weight: 0.5,
        }];
        let own = relative_chroma(muted);
        let primary_r = 0.9;
        let at = |coherence| {
            resolve_hue_slot(
                HueSlot::Red,
                &clusters,
                primary_r,
                &GenParams {
                    vibrancy_coherence: coherence,
                    ..GenParams::default()
                },
            )
        };
        let keep = at(0.0);
        assert!(keep.matched);
        assert!((keep.hue - 30.0).abs() < 1e-2);
        assert!((keep.relative_chroma - own).abs() < 1e-4);
        assert!((at(1.0).relative_chroma - primary_r).abs() < 1e-4);
        let half = at(0.5).relative_chroma;
        assert!((half - (own + primary_r) / 2.0).abs() < 1e-4);
    }

    #[test]
    fn primary_vibrancy_is_floored_for_near_gray_wallpapers() {
        // A barely-chromatic accent would otherwise make every ANSI hue
        // near-gray and indistinguishable from each other.
        assert!((primary_vibrancy([0.5, 0.005, 40.0]) - MIN_RELATIVE_CHROMA).abs() < 1e-6);
        let vivid = pure_corner_oklch([0, 255, 0]);
        assert!(primary_vibrancy(vivid) > 0.95);
    }

    #[test]
    fn ansi_ramps_match_primary_vibrancy_on_a_single_hue_wallpaper() {
        // The Hollow Knight case: dark muted teal everywhere plus one neon
        // green -- no red/blue/magenta at all. Those fallback slots used to
        // come out pastel (capped by the muted mean chroma and damped by
        // rotation distance); they should now sit at a fixed fraction
        // (fallback_vibrancy) of the highlight ramp's vibrancy.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.22, 0.035, 215.0]),
                weight: 0.8,
            },
            Cluster {
                oklab: oklch_to_oklab([0.81, 0.186, 151.0]),
                weight: 0.2,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.34,
            mean_c: 0.065,
            chromatic_mean_c: 0.065,
            is_dark: true,
        };
        let params = GenParams::default();
        let p = build_primitives(&clusters, &stats, &params);
        let primary = relative_chroma(p.highlight[&500].oklch);
        let expected = primary * params.fallback_vibrancy;
        for ramp in [&p.red, &p.blue, &p.magenta] {
            let r = relative_chroma(ramp[&500].oklch);
            assert!((r - expected).abs() < 0.02, "r={r} expected={expected}");
        }
    }

    #[test]
    fn pick_vivid_accent_ignores_vivid_cluster_below_min_weight() {
        // A tiny, spuriously-vivid cluster (compression artifact, a couple
        // stray pixels) shouldn't outrank a properly-sized chromatic
        // cluster just because it's more saturated.
        let clusters = vec![
            Cluster {
                oklab: oklch_to_oklab([0.3, 0.03, 100.0]),
                weight: 0.3,
            },
            Cluster {
                oklab: oklch_to_oklab([0.5, 0.1, 40.0]),
                weight: 0.001,
            },
        ];
        let lch = pick_vivid_accent(&clusters, 0.005);
        assert!((lch[2] - 100.0).abs() < 1e-2);
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
        // Neutral ramp skips chroma_taper (see generate_neutral_ramp) --
        // its chroma is tiny by design, well under real gamut limits.
        assert!((n500.oklch[1] - params.neutral_tint_chroma).abs() < 1e-4);
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
    fn neutral_ramp_extreme_steps_keep_full_chroma_unlike_chromatic_ramps() {
        // ansi_color0/background resolve to the neutral ramp's most extreme
        // step (950 for dark wallpapers) -- chroma_taper crushing chroma
        // there (as it correctly does for chromatic ramps, to avoid gamut
        // clipping) would defeat neutral_accent_influence at bg/color0
        // specifically, since that's exactly where it bites hardest. The
        // neutral ramp should carry its full anchor chroma at step 950,
        // unreduced by the taper that the chromatic ramps still apply.
        let neutral = generate_neutral_ramp([0.5, 0.1, 40.0], 0.1);
        let chromatic = generate_absolute_ramp([0.5, 0.1, 40.0], 0.1, 0.7);
        assert!((neutral[&950].oklch[1] - 0.1).abs() < 1e-4);
        assert!(neutral[&950].oklch[1] > chromatic[&950].oklch[1] + 0.03);
    }

    #[test]
    fn neutral_ramp_ignores_hue_lightness_bend() {
        // A green- or amber-tinted neutral must land on the same plain
        // lightness curve as any other: the bend is for vivid ramps only.
        for hue in [
            HueSlot::Yellow.anchor_hue(),
            HueSlot::Green.anchor_hue(),
            260.0,
        ] {
            let neutral = generate_neutral_ramp([0.5, 0.02, hue], 0.02);
            for &step in RAMP_STEPS.iter() {
                let l = neutral[&step].oklch[0];
                assert!(
                    (l - lightness_for_step(step)).abs() < 1e-4,
                    "hue={hue} step={step}"
                );
            }
        }
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
