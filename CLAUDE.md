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
  `red`/`yellow`/`green`/`cyan`/`blue`/`magenta`/`neutral`/`accent`, each a
  `BTreeMap<u16, Swatch>` keyed by step `50..950` (lighter → darker). Unlike
  PrimeNG's static preset ramps, these are generated fresh per wallpaper.
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
   - `generate_ramp`: fixed lightness curve per step (50→L≈0.95 down to
     950→L≈0.15, **same curve for every hue** — see "Known rough edges"
     below), chroma capped by `stats.mean_c * chroma_clamp_factor` and
     tapered near the lightness extremes to reduce gamut clipping.
   - `neutral` ramp: near-zero chroma, hue = weighted circular mean hue
     across all clusters (weighted by `weight * chroma` so gray clusters
     don't skew the estimate) — gives bg/fg a faint image-matched tint
     instead of true gray.
   - `accent` ramp: the single highest-weight cluster, hue-unlocked — this
     is deliberately *exactly* what matugen extracts, just demoted to one
     token among many instead of the seed for the whole palette.
5. `semantic::resolve` — parse `DEFAULT_SEMANTIC_TOML` (or user override via
   `Config::semantic` path), look up each `"ramp.step"` / `auto:*` value
   against the primitives, produce concrete `Swatch`es.

## Known rough edges (expected iteration, not bugs to silently "fix")

- The lightness-per-step curve is hue-agnostic. Pure yellow is naturally
  very light (Oklch L≈0.97) so yellow's mid-ramp steps read as olive/brown
  rather than vivid yellow — correct color science, but may not match
  intuitive "ANSI yellow" expectations. If tuning this, consider a
  per-hue lightness curve rather than a shared one.
- Gamut handling is naive clamping (`color::oklab_to_srgb_u8` clips r/g/b to
  `[0,1]` after conversion) rather than proper gamut mapping (reducing
  chroma until in-gamut). Fine for v1; revisit if clipped colors look
  visibly desaturated/wrong on saturated wallpapers.
- `fallback_rotate`'s confidence damping (`* 0.8` chroma) is an arbitrary
  constant, not derived from anything. Tune by eye.

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

These constants (`hue_tolerance`, `chroma_clamp_factor`, `neutral_tint_chroma`,
`min_cluster_weight`) are the knobs — see `Config` in `src/config.rs` and
`GenParams` in `src/palette_gen.rs`. `mise run preview -- ./wallpaper.jpg`
is the fast loop for eyeballing changes; there's no automated "looks good"
check because that's inherently a perceptual judgment call.

Every `Config` field is tunable via a TOML file rather than editing
constants in source — see "Config file" below. `hue_tolerance` and
`chroma_clamp_factor` are the ones actually worth iterating on per
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

- `src/color.rs` — sRGB↔Oklab↔Oklch conversions, gamut clip, hex, circular
  hue distance, pure-primary hue anchor computation. Pure math, unit tested.
- `src/extract.rs` — image loading/downsampling, k-means, image stats
  (`mean_c` vs `chromatic_mean_c`, see rough-edges fix above). Unit tested
  (cluster separation, weight normalization, chromatic_mean_c behavior).
- `src/palette_gen.rs` — hue matching/fallback (fallback picks by chroma
  among clusters clearing `min_weight`, not by weight), ramp generation,
  primitives assembly. Unit tested (monotonic ramps, chroma cap, hue
  matching/fallback correctness including the vivid-vs-prevalent case,
  anchor spread).
- `src/semantic.rs` — default mapping, resolution logic. Unit tested
  (parses, resolves, `auto:bg`/`auto:fg` pick correctly, unknown ramp errors).
- `src/config.rs` — `Config` (TOML-loadable, has `Default`), XDG-aware
  `load`, `annotated_default_toml` template, maps to `GenParams`.
- `src/xdg.rs` — XDG Base Directory config path resolution. Unit tested.
- `src/preview.rs` — `print_ansi16` (raw swatch blocks) and
  `print_terminal_mock` (prompt/`ls`/log-levels/diff/code-line mockup using
  the resolved bg/fg/ansi colors together) — isolated color blocks made it
  hard to judge fg-on-bg contrast and overall feel, the mockup is meant to
  approximate "does this look like a real terminal."
- `src/main.rs` — clap CLI (`generate`, `preview`, `config init`/`path`,
  `clusters` for raw k-means diagnostics), wires the pipeline.

## Testing

`mise run test` (21 unit tests as of the accent-detection fix, all
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
