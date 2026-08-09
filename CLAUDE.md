# a16p-gen — project context

Personal Hyprland ricing tool. Rust CLI, mise-managed toolchain (see
`mise.toml` for tasks — always prefer `mise run <task>` over raw `cargo`
invocations when a task exists).

## Problem this solves

matugen picks one accent color from a wallpaper and derives the *entire*
scheme (including ANSI16) as shades of it — color1 (red/error) and color2
(green/success) end up as barely-distinguishable tints of the same hue.
wallust spreads ANSI16 across real hues but the lightness/saturation doesn't
track the wallpaper, so output looks visually disconnected from the image.

This tool fixes the ANSI16 generation step specifically: every chromatic
slot keeps its semantic hue family (red stays red-ish, green stays
green-ish) *and* stays anchored to what the wallpaper's own lightness/chroma
actually looks like.

Full design rationale and the original decision log live in the approved
plan (language choice, token-tier design, algorithm steps) — if it's still
around at `~/.config/claude/plans/`, it has more color-science backstory
than this file. This file is the living/current-state doc; that one is a
point-in-time record of the initial planning conversation.

## Scope

v1 (current) = palette engine only: image in, color tokens out
(`primitives.json` / `semantic.json` / terminal swatch). No templating, no
dotfile installation, no hooks yet — those are deliberately deferred until
the palette algorithm itself is validated as good against real wallpapers.
Don't build templating/install machinery unless asked; it's a known, planned
v2, not forgotten scope.

## Token model (PrimeNG-inspired three tiers)

- **primitive** (`src/palette_gen.rs::Primitives`): generated ramps —
  `red`/`yellow`/`green`/`cyan`/`blue`/`magenta`/`neutral`/`accent`/
  `highlight`, each a `BTreeMap<u16, Swatch>` keyed by step `50..950`
  (lighter → darker). Unlike PrimeNG's static preset ramps, these are
  generated fresh per wallpaper. `accent` vs `highlight`: prevalence vs
  vividness, see pipeline section below — this split exists because they
  gave visibly different, both-useful answers once tested on a real
  wallpaper, not because it was planned upfront.
- **semantic** (`src/semantic.rs`): role → `"ramp.step"` string mapping,
  e.g. `ansi_color1 = "red.500"`, `danger = "red.500"`. This is where the
  ANSI semantic contract becomes explicit, editable data instead of an
  emergent (and in matugen's case, broken) property of the generation
  algorithm. `auto:bg`/`auto:fg` are special values resolved against the
  detected dark/light mode (`ImageStats::is_dark`) rather than a fixed step,
  so the same mapping works for both light and dark wallpapers.
- **component** (not built): future per-app tokens (`waybar.module.active`)
  resolving through semantic → primitive. This is what the (not-yet-built)
  template engine will consume.

## Pipeline (src/main.rs::run_pipeline)

1. `extract::load_and_sample` — downsample image (`max_dim`, default 200px
   longest side), convert every pixel to Oklab.
2. `extract::kmeans_oklab` — hand-rolled Lloyd's k-means in Oklab space
   (`k` clusters, default 16), farthest-point seeding (deterministic, no
   `rand` dependency). Returns weighted `Cluster { oklab, weight }`.
3. `extract::image_stats` — weighted mean L/C across clusters, dark/light
   mode via `mean_l < 0.55`.
4. `palette_gen::build_primitives`:
   - For each of the 6 chromatic `HueSlot`s: compute the *true* Oklch hue
     angle of the pure sRGB primary/secondary (`HueSlot::anchor_hue`, not
     hardcoded — computed via actual color conversion so it's exact for
     whatever the `palette` crate implements), then `match_hue` (nearest
     cluster within `hue_tolerance` degrees, weight ≥ `min_cluster_weight`)
     or `fallback_rotate` (rotate the most prominent chromatic cluster onto
     the target hue, damping confidence via reduced chroma) if nothing
     matches.
   - `generate_ramp`: lightness curve per step (50→L≈0.95 down to
     950→L≈0.15), bent per-hue toward each hue's own peak-chroma lightness
     (`hue_lightness_bend`, see "Fixed" below), chroma capped by
     `stats.mean_c * chroma_clamp_factor` and tapered near the ramp's edges
     to reduce gamut clipping.
   - `neutral` ramp: hue/chroma blended between a flat baseline (weighted
     circular mean hue across all clusters, fixed `neutral_tint_chroma`)
     and the wallpaper's *vivid* accent (`pick_vivid_accent`), via
     `neutral_accent_influence` (0 = flat baseline exactly, 1 = fully
     accent-tinted). Default 0 — opt-in, since how much personality bg/fg
     should have is a judgment call, and turning it up too far reintroduces
     matugen's "everything is one hue" problem, just relocated to bg/fg.
     `chroma_taper` (see below) means the effect is subtle right at the
     ramp extremes (`background`/`foreground` = steps 950/50) by design —
     it's much more visible mid-ramp (`surface_card`=900, `surface_border`=700).
   - `accent` ramp (`pick_accent`): the single highest-weight cluster,
     hue-unlocked — this is deliberately *exactly* what matugen extracts.
     For a mostly-dark/muted wallpaper, the most-prevalent color is often
     just dark/gray, not a color at all — `accent` is kept for anyone who
     wants literal matugen parity, but nothing in the default semantic
     mapping points at it.
   - `highlight` ramp (`pick_vivid_accent`): the most *vivid* cluster
     clearing `min_cluster_weight`, not the most prevalent one — same
     reasoning as the `fallback_rotate` fix below, and shares its
     `most_vivid` helper. This is what `primary` maps to by default, and
     what the preview's "accent" swatch shows.
5. `semantic::resolve` — parse `DEFAULT_SEMANTIC_TOML` (or user override via
   `Config::semantic` path), look up each `"ramp.step"` / `auto:*` value
   against the primitives, produce concrete `Swatch`es.

## Known rough edges (expected iteration, not bugs to silently "fix")

None currently tracked — the three rough edges below were the initial
batch (identified 2026-08-09), and all three are fixed. Add new ones here
as they turn up; don't silently "fix" something noted here without calling
it out, since it may be a deliberate v1 tradeoff rather than a bug.

### Fixed: `fallback_rotate`'s confidence damping was a flat, unexplained constant

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

### Fixed: naive per-channel gamut clamping instead of proper gamut mapping

`color::oklab_to_srgb_u8` used to clip r/g/b independently to `[0,1]` after
conversion. That shifts hue and perceived lightness by an uncontrolled
amount, since each channel gets clamped by a different fraction — visibly
wrong on saturated wallpapers, and more likely to bite after the
`hue_lightness_bend` fix above, since bending deliberately pushes some
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
(above) seemed to hit an unfixable "asymptote" and got abandoned in favor
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

### Fixed: hue-agnostic lightness curve made vivid yellow read as olive/brown

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

### Fixed: small vivid accents got crushed by weight-based selection

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

### Fixed: `primary`/preview accent was gray on the same wallpaper this whole fix was about

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

### Accent-tinted bg/fg (`neutral_accent_influence`)

One of matugen's actually-liked traits was accent-derived backgrounds
(vs. this tool's originally-flat neutral gray). Rather than rebuild that
as a different architecture, it's the same primitive/semantic model: the
`neutral` ramp's generation was extended to optionally blend toward
`pick_vivid_accent` (see pipeline section above). No new token tier, no
branch needed — this is why the three-tier model was worth having.

These constants (`hue_tolerance`, `chroma_clamp_factor`, `neutral_tint_chroma`,
`min_cluster_weight`, `neutral_accent_influence`, `hue_lightness_bend`) are
the knobs — see `Config` in `src/config.rs` and `GenParams` in
`src/palette_gen.rs`. `mise run preview -- ./wallpaper.jpg` is the fast loop
for eyeballing changes; there's no automated "looks good" check because
that's inherently a perceptual judgment call.

Every `Config` field is tunable via a TOML file rather than editing
constants in source — see "Config file" below. `hue_tolerance`,
`chroma_clamp_factor`, `min_cluster_weight`, `neutral_accent_influence`, and
`hue_lightness_bend` are the ones actually worth iterating on per
wallpaper; the rest rarely need touching.

## Config file (XDG)

`Config::load` (`src/config.rs`) resolves, in order: explicit `--config`
path (must exist) → `$XDG_CONFIG_HOME/a16p-gen/config.toml` (fallback
`~/.config/a16p-gen/config.toml`, via `src/xdg.rs::default_config_path`,
Linux-only plain env lookup, no `dirs` crate) → `Config::default()`
silently if neither exists. The whole-struct `#[serde(default)]` means a
config file only needs to set the fields being tuned; everything else
falls back.

`a16p config init` writes `Config::annotated_default_toml()` — a
comment-per-field template with defaults interpolated from
`Config::default()` (can't drift out of sync) — to the XDG path; refuses
to overwrite without `--force`. `a16p config path` prints the resolved
path without touching anything. Mise wraps these as `config-init` /
`config-path`.

## Module map

- `src/color.rs` — sRGB↔Oklab↔Oklch conversions, gamut mapping
  (`gamut_map_oklch`, chroma-reduction not per-channel clipping -- see
  "Fixed" above), hex, circular hue distance, pure-primary corner
  computation. Pure math, unit tested.
- `src/extract.rs` — image loading/downsampling, k-means, image stats
  (`mean_c` vs `chromatic_mean_c`, see rough-edges fix above). Unit tested
  (cluster separation, weight normalization, chromatic_mean_c behavior).
- `src/palette_gen.rs` — hue matching/fallback (fallback picks by chroma
  among clusters clearing `min_weight`, not by weight; damps confidence by
  rotation distance, see "Fixed" above), ramp generation, primitives
  assembly. Unit tested (monotonic ramps, chroma cap, hue matching/fallback
  correctness including the vivid-vs-prevalent case and confidence damping,
  anchor spread).
- `src/semantic.rs` — default mapping, resolution logic. Unit tested
  (parses, resolves, `auto:bg`/`auto:fg` pick correctly, unknown ramp errors).
- `src/config.rs` — `Config` (TOML-loadable, has `Default`), XDG-aware
  `load`, `annotated_default_toml` template, maps to `GenParams`.
- `src/xdg.rs` — XDG Base Directory config path resolution. Unit tested.
- `src/preview.rs` — `print_ansi16` (`background`/`accent`/`foreground` row,
  where accent = `resolved["primary"]`, then the raw ANSI16 swatch) and
  `print_terminal_mock` (prompt/`ls`/log-levels/diff/code-line mockup using
  the resolved bg/fg/ansi colors together) — isolated color blocks made it
  hard to judge fg-on-bg contrast and overall feel, the mockup is meant to
  approximate "does this look like a real terminal."
- `src/main.rs` — clap CLI (`generate`, `preview`, `config init`/`path`,
  `clusters` for raw k-means diagnostics), wires the pipeline.

## Testing

`mise run test` (37 unit tests as of the fallback-confidence fix, all
pure-function — no image fixtures needed). For pipeline-level sanity
checks, synthetic test images were generated with Python/Pillow (not
committed, were scratch files) — a colorful patchwork image to verify
direct hue matching, and a warm-only gradient to verify the fallback path
doesn't crash or produce garish output. Recreate similarly if needed
rather than relying on `find`-ing real wallpapers. For real-wallpaper
regressions, `a16p clusters <image>` plus `a16p preview <image>` against
`~/media/pictures/wallpapers/*` is the actual test bed — synthetic images
didn't surface the weight-vs-vividness bug at all, only a real stylized
wallpaper did.

When testing XDG path resolution by hand, override `XDG_CONFIG_HOME`
directly — overriding `HOME` alone does nothing if `XDG_CONFIG_HOME` is
already set in the shell (it takes precedence, correctly), and `mise` runs
inherit the calling shell's env. Confirmed the hard way: an early manual
test overrode `HOME` only and ended up writing a real file to this user's
actual `~/.config/a16p-gen/config.toml` as a side effect.
