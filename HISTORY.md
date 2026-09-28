# a16p-gen — fixed rough edges

Full narrative log of algorithm bugs found and fixed during development.
Split out of `CLAUDE.md` (2026-08-11) to keep that file lean — `CLAUDE.md`
keeps a one-line pointer per entry; this file has the full "what broke, why,
how it was confirmed, how it was fixed" story for each. Git log/commit
messages have the diffs; this has the reasoning that doesn't fit a commit
message or a code comment.

### `fallback_rotate`'s confidence damping was a flat, unexplained constant

Chroma for a rotated fallback color (no cluster matched within
`hue_tolerance` of a target ANSI hue, so the most vivid qualifying cluster
gets rotated onto that hue instead) was always damped by a flat `* 0.8`,
regardless of how much of a stretch the rotation actually was. A cluster
just 1 degree past `hue_tolerance` (`match_hue` almost accepted it) got
damped exactly as hard as one dragged from the opposite side of the hue
wheel — no signal in the number, "tune by eye" per the old rough-edges note.

Fixed by `fallback_confidence(distance, tolerance)`: linearly scales
confidence from 1.0 (distance == tolerance, i.e. barely missed a real
match) down to `FALLBACK_MIN_CONFIDENCE` (0.5) at distance == 180 (the
opposite hue, the worst possible rotation). The case where `most_vivid`
finds no qualifying cluster at all (pure stats-based guess, not even a real
cluster's hue to measure a distance from) gets the floor confidence
outright rather than a computed value. `fallback_rotate` gained a
`tolerance` parameter to make this possible — same position as
`match_hue`'s, for symmetry.

Confirmed this isn't just theoretical: `a16p clusters
~/media/pictures/wallpapers/isma-defiance.jpg` shows the nearest cluster to
the yellow anchor (109.8°) sits at hue 142.7° — distance ~33°, just past the
default `hue_tolerance=30`. That's exactly the near-miss case this fix
targets; it now keeps ~99% confidence instead of a flat 80%.

### Naive per-channel gamut clamping instead of proper gamut mapping

`color::oklab_to_srgb_u8` used to clip r/g/b independently to `[0,1]` after
conversion. That shifts hue and perceived lightness by an uncontrolled
amount, since each channel gets clamped by a different fraction — visibly
wrong on saturated wallpapers, and more likely to bite after the
`hue_lightness_bend` fix below, since bending deliberately pushes some
hues' mid-ramp steps toward lightnesses where their available chroma is
lower.

Fixed by `color::gamut_map_oklch`: if an Oklch color is out of the sRGB
gamut, binary-search its chroma down (holding L and hue fixed) until it
isn't, the standard "hold L/H, back off C" gamut-mapping approach.
`oklab_to_srgb_u8` now gamut-maps before converting; the old per-channel
`.clamp(0.0, 1.0)` stays as a final float-rounding safety net, which should
be a no-op in practice.

Finding this needed one real bug fix first: the initial `in_gamut` check
used `oklab.into_color()`, `palette`'s "safe" conversion trait — which
already clamps its output internally, making an in-`[0,1]` check on its
result a tautology that's always true. That's also why an earlier attempt
at a numeric peak-chroma-lightness search for `peak_lightness_for_hue`
(below) seemed to hit an unfixable "asymptote" and got abandoned in favor
of 6-point interpolation — same root cause, not a real math limitation.
Confirmed by testing pure red's own known boundary chroma (~0.258 at its
own L≈0.628): `into_color()`-based `in_gamut` reported chroma 0.5 as
in-gamut at that same L (wrong); `into_color_unclamped()` correctly finds
the real crossing. `gamut_map_oklch`'s binary search uses
`into_color_unclamped()` and is bounded at `hi=original_chroma` (chroma 0
is always in-gamut, so `[0, original_chroma]` always brackets a real
crossing) — the interpolation approach for `peak_lightness_for_hue` was
kept as-is regardless, since it's simpler/cheaper and accurate enough for
a curve bend, not a precision gamut map.

### Hue-agnostic lightness curve made vivid yellow read as olive/brown

Every ramp step was forced through the same L(step) curve regardless of
hue — but each hue reaches its highest achievable chroma at a different
lightness (pure yellow peaks near Oklch L≈0.97, pure blue near L≈0.45).
Forcing yellow's step 500 down to the shared curve's L≈0.53 meant its
chroma got clipped hard by the sRGB gamut boundary at that lightness,
rendering as muddy olive/brown (`#856900`-ish) even when the wallpaper's
actual yellow was vivid.

Fixed by `hue_lightness_bend` (`GenParams`/`Config`, default 0.7, unlike
`neutral_accent_influence` this is a correctness fix so it defaults on, not
opt-in): `palette_gen::lightness_for_step_bent` reparametrizes the ramp's
`t` position through a two-segment piecewise-linear warp anchored at step
500, so step 500's lightness moves toward `peak_lightness_for_hue(hue)`
instead of the generic curve. The true curve endpoints (t=0/L=0.95, and
exactly step 950/L=0.15) stay fixed and the result stays monotonic by
construction — a quadratic fit through 3 points was tried first and
rejected because it can overshoot `[0.15, 0.95]` or wobble non-monotonically
when the peak sits close to an endpoint (yellow's does).

`peak_lightness_for_hue` estimates the peak by piecewise-linear
interpolation between the 6 `HueSlot` corners' own computed L
(`HueSlot::anchor_lch`), not a numeric gamut-boundary search — a real
per-hue gamut search (binary-search chroma for in-gamut-ness, then
ternary-search the maximizing L) was tried and rejected: at a hue angle
that exactly matches a primary's own hue, chroma asymptotically approaches
the gamut corner without the round-tripped sRGB ever leaving `[0, 1]`, so
the boundary search never finds a crossing and returns garbage. The 6-point
interpolation sidesteps that failure mode entirely.

Bending also broke `chroma_taper`, which damps chroma near a *fixed* L=0.5
center — once bending moved yellow's step 500 to L≈0.84 (near the ramp's
light edge), the taper read that as "near an extreme" and crushed the
chroma right back down. Fixed by tapering against the step's *position* in
the ramp (`lightness_for_step`, pre-bend) rather than its final bent
lightness, decoupling "how much to damp near the ramp's edges" (tuned,
should stay stable) from "what L this hue actually renders at" (now
per-hue). See doc comments on `lightness_for_step_bent`, `generate_ramp`,
and `peak_lightness_for_hue` in `palette_gen.rs`.

### Small vivid accents got crushed by weight-based selection

Diagnosed against `~/media/pictures/wallpapers/pixel-art-hollow-knight.jpg`
(mostly black/dark-gray, a couple small vivid-orange "infection bubble"
regions, more area of muted-orange "fog"): the resulting ANSI16 was
uniformly muted, including red/orange, even after tuning
`chroma_clamp_factor` down. Root cause was weight used as the significance
proxy in three places at once:

1. `chroma_clamp_factor` was applied to `stats.mean_c` — chroma averaged
   over *all* clusters, so a 56%-black image drags the reference near zero
   regardless of how vivid the small accent actually is.
2. `match_hue`'s `min_cluster_weight` floor (previously 0.02 = 2% of
   pixels) excluded the vivid bubble clusters (weight ~0.6%/0.2%) even
   though their hue was *closer* to true red than the larger, duller "fog"
   cluster that did qualify.
3. `fallback_rotate` picked the highest-*weight* chromatic cluster, i.e.
   the same dull fog cluster again, when it should represent "the color
   that draws the eye," not "the color covering the most pixels."

Fixed by: `ImageStats::chromatic_mean_c` (weighted mean chroma over only
clusters with chroma ≥ `extract::CHROMATIC_THRESHOLD`, used for
`chroma_clamp_factor` instead of whole-image `mean_c`); lowering
`min_cluster_weight`'s default to 0.005 (k-means already absorbs real
per-pixel noise into larger clusters, so a 2% floor was excluding
legitimate small design elements, not just noise); and `fallback_rotate`
now picks the *most vivid* cluster among those clearing the `min_weight`
floor, not the most prevalent one. See the doc comments on
`fallback_rotate` and `build_primitives` in `palette_gen.rs` for the exact
reasoning — kept in-code, not just here, since it's non-obvious.

`a16p clusters <image>` (added alongside this fix) prints raw k-means
cluster weight/Oklch/hex plus the six hue anchors — the diagnostic tool
that found this. Reach for it first when a hue slot looks wrong; it shows
directly whether the issue is "no cluster near that hue" vs "a cluster
exists but got filtered/outcompeted."

### `primary`/preview accent was gray on the same wallpaper this whole fix was about

After adding `pick_vivid_accent` for the neutral-ramp blend, added an
accent swatch to `preview` (`background`/`accent`/`foreground`) pulling
from the `primary` semantic role — which still mapped to `accent.500`
(`pick_accent`, prevalence-based). For hollow-knight, the single most
*prevalent* cluster is near-black (29.5% weight, ~0 chroma), so `primary`
resolved to flat gray `#6b6b6b` — the exact vivid-vs-prevalent trap this
whole fix arc was about, just in the one token that hadn't been touched
yet. Fixed by adding the `highlight` primitive (`pick_vivid_accent`,
reusing the value already computed for the neutral blend) and repointing
`primary = "highlight.500"` in `DEFAULT_SEMANTIC_TOML`. `accent` itself
was left alone — still prevalence-based, still there if literal matugen
parity is ever wanted for something.

### `match_hue` let a weak nearest-angle cluster outrank a more vivid farther one

`match_hue` picked its candidate purely by nearest hue angle among clusters
clearing `min_weight` and `hue_tolerance` — no chroma/vividness check, and
(unlike `fallback_rotate`) no confidence damping either, so whatever it
picked got trusted at full strength. That meant a weak, barely-qualifying
cluster (technically in tolerance, but nearly gray) could beat a genuinely
vivid cluster sitting a few degrees farther away, producing a *more* muted
color than the no-match `fallback_rotate` path would've given for the same
slot — `fallback_rotate` explicitly picks by vividness (see `most_vivid`)
and damps confidence by rotation distance, so a "weak real match" could
paradoxically end up worse than "no match at all."

Confirmed on `pixel-art-neon-city.png`'s red slot: at default
`hue_tolerance=30` (no match, real cluster too far), `fallback_rotate`
picked the image's genuinely vivid purple cluster and rotated it onto red
→ chroma 0.106 (`#b6665a`, a real red). At `hue_tolerance=45`, a stray weak
cluster at hue 66° (weight 0.3%, chroma 0.034) fell into tolerance and
`match_hue` accepted it verbatim → chroma 0.032 (`#9f8e7d`, dull tan) —
strictly worse than not matching at all. This also explains why red reads
as "more responsive to tuning" than other hues: wallpapers commonly have a
real strong warm cluster near red's target hue, so red usually gets a
genuine high-chroma match; yellow/cyan/magenta more often only have weak
clusters nearby and were exposed to this bug. Widening `hue_tolerance` to
fix muted colors reliably made this worse, not better, since it let more
weak clusters qualify as full-confidence "real" matches.

Fixed by having `match_hue` pick the highest-chroma candidate among
tolerance-qualifying clusters instead of the nearest-angle one — mirrors
the vividness-first logic `most_vivid`/`fallback_rotate` already use, so
a "real match" can no longer be a worse pick than a fallback rotation
would've been.

### `chroma_taper` crushed `neutral_accent_influence` specifically where it mattered most

`ansi_color0`/`background` (`auto:bg`) resolve to the neutral ramp's most
extreme step (950 for dark wallpapers, i.e. the darkest). `chroma_taper`
damps chroma hardest exactly at the ramp's extremes (950/50) — correct for
*chromatic* ramps, where it exists to avoid relying on out-of-gamut chroma
that would otherwise get clipped. But it was applied uniformly to the
`neutral` ramp too, whose chroma is tiny by design
(`neutral_tint_chroma` default 0.015, an order of magnitude under any real
hue's gamut boundary ~0.2+) and was never at clipping risk in the first
place. Net effect: turning up `neutral_accent_influence` barely moved
`background`/`ansi_color0` at all, while `ansi_color7`/`ansi_color8`
(mid-ramp steps 300/600, where the taper barely bites) visibly showed the
tint — same knob, wildly different visible effect depending on which step
a role happens to sit at, with the single most visually prominent token
(terminal background) being the one that stayed frozen.

Fixed by giving the neutral ramp its own path (`generate_neutral_ramp`)
that skips `chroma_taper` entirely, keeping its full anchor chroma at every
step. `gamut_map_oklch` (still applied downstream in `swatch_from_oklch`)
remains the real safety net for genuine out-of-gamut colors, so this
doesn't reopen the clipping problem the taper was originally added for —
confirmed by testing `neutral_accent_influence` 0.1→0.4 on a real
wallpaper: `background` visibly shifted hue/saturation instead of staying
pinned near-black.

### ANSI ramps read pastel next to a neon primary (absolute chroma across hues)

On `videogame-hollowknight-1.jpg` (dark muted teal, one vivid acid green,
no red/blue/magenta), `primary` (`highlight.500`) was #4ed879 while
`red.500` was #a4736b — neon vs. pastel. Measured as chroma over the max
in-gamut chroma at that step's L and hue (`color::relative_chroma`):

| ramp | 500 | C / Cmax(L,h) |
|---|---|---|
| highlight | #4ed879 | 0.83 |
| cyan / green / yellow | — | 0.61 / 0.50 / 0.46 |
| red / blue / magenta | #a4736b / #485b80 / #a388a1 | 0.26 / 0.24 / 0.16 |

Three stacked causes: (1) hue-slot ramps were capped at an *absolute*
`chromatic_mean_c * chroma_clamp_factor`, dragged down by the large muted
teal, while `highlight` bypassed that cap; (2) `fallback_rotate` capped
again at `chromatic_mean_c*1.5` and damped by `fallback_confidence` (×0.67
for red here); (3) one absolute C is a very different share of the gamut
per hue — 0.064 is pastel for red/magenta but fairly strong for cyan,
which is why magenta looked the most washed out.

Fixed by moving every chromatic ramp to a relative-chroma model. The
primary's (vivid accent's) relative chroma `r_p` (`primary_vibrancy`,
floored at `MIN_RELATIVE_CHROMA` = 0.25 so ANSI hues stay tellable apart
on near-gray wallpapers) is the target. Unmatched slots take `r_p` at their
exact anchor hue; matched slots keep their cluster's hue and blend its own
relative chroma toward `r_p` by the new `vibrancy_coherence` knob (default
0.7 — lift a muted matched blue most of the way, keep a hint of the
wallpaper's softness). `generate_ramp` then gives every step that fraction
of `max_chroma` at the step's own L, times `chroma_taper` normalized to 1 at
step 500. `highlight` uses the same model with the accent's unfloored
vibrancy, so `primary` and ANSI 500s share one scale.

This **supersedes the `fallback_confidence` fix** (first entry above):
`fallback_rotate`/`fallback_confidence`/`FALLBACK_MIN_CONFIDENCE` are gone.
Once the ramp's lightness curve overrode the borrowed L anyway, chroma
damping was the rotation's whole visible effect — exactly the pastel
problem. A fallback's lower confidence now only shows as "exact anchor hue,
no wallpaper hue shift". `chroma_clamp_factor` still exists but now only
caps the neutral accent tint and the matugen-parity `accent` ramp.

After, same wallpaper: all hue 500s at 0.80 (green 0.84, matched), primary
#56d77c at 0.80, red #df4032. Tradeoff seen on the other test wallpapers:
a single very vivid accent (castle's amber at 0.96, witcher's orange at
0.93) now drives the whole ANSI set near the gamut edge — close to pure
RGB primaries. Consistent with the goal ("same feel as the primary") but
louder than wanted once looked at in a real terminal: on Witcher (orange
primary, `hue_tolerance = 35`), `magenta` — absent from the wallpaper
entirely — came out `#e52be4`/`#ee5fec` (400), the loudest color in the palette.

Follow-up: `fallback_vibrancy` (default 0.75) scales the primary's
vibrancy for unmatched slots only (still floored at `MIN_RELATIVE_CHROMA`).
A flat scale was chosen over a ceiling on `primary_vibrancy`: a ceiling
would flatten every loud wallpaper to the same fallback vibrancy and stop
tracking the primary, the whole point of the change. Witcher magenta
0.93 → 0.69 (`#d357d0`); Hollow Knight fallbacks 0.80 → 0.60 (red
`#ca584a`) — still far from the original 0.26. Matched slots that read too
loud (Witcher's sky-matched blue/cyan at 0.84) are `vibrancy_coherence`'s
job: 0.4 brings them to ~0.75.

The same session surfaced a separate problem with widening `hue_tolerance`
— see the next entry.

### Wide `hue_tolerance` let a slot take a neighboring hue outright

A matched slot used the cluster's hue as-is. The ANSI anchors are unevenly
spaced (red→yellow 80.6°, yellow→green **32.7°**, green→cyan 52.3°,
cyan→blue 69.3°, blue→magenta 64.3°, magenta→red 60.8°), so any tolerance
near half the smallest gap breaks the red≠orange / yellow≠green contract.
Tolerance was being widened for a good reason: Witcher's sky (225–232°)
sits almost exactly between the cyan and blue anchors, so no tolerance
could both reach it and stay safe. At 35: Hollow Knight's `yellow`
matched acid green (~34° off) and came out green; Witcher's `red` became
the orange primary and `cyan`/`blue` both took the same sky cluster.

Fixed in two parts. `hue_shift_limit` (default 0.3) caps how far a matched
slot's hue moves toward its cluster, as a fraction of the gap to the
neighbor on that side (`clamp_hue_shift`) — a fixed degree cap can't work
when 15° is small for red→yellow but half of yellow→green. Below 0.5 two
slots can never cross (tested). And `match_hue` now ranks own-family
clusters (nearer this slot's anchor than any other) above neighbor-family
ones before comparing chroma, so a vivid green can't displace a real, softer
yellow.

The first version measured red's gap to yellow directly. On
`pixel-art-castle` that let red shift 22.5° to h52 — `red.500` came out
**identical to `primary`** (`#e47317`), so `danger` was the accent color.
The red→yellow gap holds a whole named color of its own, so pure sRGB
orange (`#ff8000`) is now an extra boundary in `neighbor_gaps` (not a
slot): red and yellow each measure to it. Red then moves at most ~8°
(h36, `#e84913`), yellow ~16°. Lowering the limit globally was the
alternative, but red was still red-orange at 0.15 and that also took away
blue's lean toward the sky. No boundary for the other wide gaps on purpose
— Oklch packs azure/violet near blue, and a sky-tinted blue is character
worth keeping.

Result at `hue_tolerance` 20/30/35 on all three test wallpapers: ANSI
500 hues stay in wheel order with a ≥28° minimum gap. Witcher cyan/blue
split the sky (216°/243°). Hollow Knight yellow `#bed664` (h120).

### Neutral ramp was hue-bent, so `ansi_color8` read as a mid gray

Color 8 ("bright black", `neutral.600` in dark mode) looked too bright —
closer to a mid gray than to color 0. Cause: `generate_neutral_ramp` passed
`hue_lightness_bend` through like the chromatic ramps. The bend moves step
500 toward the lightness where the ramp's *hue* peaks in chroma — the fix
for yellow reading olive — but a near-gray ramp has no chroma to preserve,
so it only lifted the middle of the ramp. The test wallpapers' tints are
green/amber, whose peaks sit at L≈0.8–0.9: `neutral.600` landed at
L 0.60–0.66 (5–6:1 on the background) instead of the plain curve's 0.445,
and every neutral step drifted by up to 0.05 L depending on the wallpaper.

Remapping `ansi_color8` to `neutral.700` was tried first (L 0.47–0.51) and
would have worked for color 8 alone, but it treated a symptom: the same lift
pushed `text_muted_color`/`ansi_color7`/surfaces around. Fixed at the root
instead — the neutral ramp now skips the bend and follows the plain
lightness curve on every wallpaper. `ansi_color8` → L 0.44, ~2.6:1 on bg.

That darkened the whole middle of the ramp, so two semantic roles were
remapped to keep usable contrast: `text_muted_color` dark 500 → 400
(3.7 → 5.3:1; light stays 700, ~8.2:1) and `surface_border` 700/300 →
600/400 (dark 1.8 → 2.6:1, light 2.0 → 2.8:1). `ansi_color7` now reads as
a clearly dimmed foreground (L 0.70, ~7.3:1) instead of nearly matching it
(L 0.86 vs fg 0.93). Side effect: the ramp is now near-symmetric in
contrast (500 ≈ 3.7:1 dark / 4.0:1 light), so the old "neutral ramp is
lopsided" note on `text_muted_color` no longer applied.

### `hue_lightness_bend` pushed blue below readable contrast

Blue text was nearly unreadable on dark backgrounds: `ansi_color4` came out
at L 0.48, ~2.8:1 (`#2451c1`) on both pixel-art wallpapers and
hollowknight-3. Blue was the only ANSI slot under 4.5:1. The bend
moves step 500 toward the lightness where each hue peaks in *chroma*. That
lifted yellow (peak L≈0.97) but pulled pure blue (peak L≈0.45) *below*
the generic curve's 0.53. The bend chases vividness, and that only helped
readability for hues that peak light.

Shifting the blue anchor from 264° toward 200–210° was considered and
rejected. Contrast depends almost entirely on L: at equal L and gamut
fraction, h264/h240/h210 measure 4.8/5.0/5.2:1 at L 0.60. A lower hue would
only have helped indirectly, because `peak_lightness_for_hue` interpolates
toward cyan's high peak. Meanwhile blue would sit 10–15° from cyan (195°)
and read teal at 210°, which would have forced cyan toward green in turn.
Common themes keep blue at h245–264 and raise L instead (Tokyo Night
`#7aa2f7` h264/L0.72, Catppuccin `#89b4fa` h260/L0.77).

Fix: `min_mid_lightness` (default 0.62) floors step 500 after the bend,
through the same monotonic t-warp. It applies to the six chromatic ramps
and `highlight` (`primary`), but not to `accent` (matugen parity) or
`neutral` (plain curve on purpose, see above). Result on the six test
wallpapers: blue.500 is L 0.62 at 5.3–5.5:1 (`#5c83d7` on pixel-art-castle),
hues unchanged, and the wallpaper's azure lean (243°) is kept. Red on the
Hollow Knight wallpapers moved L 0.60 → 0.62 (4.6–4.8 → 5.0–5.2:1). No
other slot moved. Hollowknight-3's `primary` is its blue, so it lifted too
(`#3d59a5` → `#6e85bc`).

Tradeoff: in `mode = "light"` blue was the one chromatic 500 that read
fine on a light background. It now reads ~2.8:1 there, the same as red and
magenta. This adds to the existing light-mode open item. It does not
create a new one.
