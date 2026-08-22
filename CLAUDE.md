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

v1 = palette engine (`primitives.json` / `semantic.json` /
`component.json` / terminal swatch) *plus* templating: `a16p render`
(`src/render.rs`) renders per-application config templates with the
generated colors via minijinja. This slice is done and considered
mature — no known gaps in it. Still no dotfile *installation* (nothing
manages symlinks/copies into place, discovers which apps are installed,
or manages a template collection for the user) and no hooks beyond a
template's own `post_hook` — those remain deliberately deferred.

What's left in v1: polishing the palette generation algorithm itself
(hue matching, ramp curves, neutral tinting, gamut mapping — the knobs
listed under "Known rough edges" and in `Config`/`GenParams`) against
real wallpapers, no architectural changes expected. After that, v2 is
planned as a substantially larger feature set (dotfile installation,
template collection management, etc.) — scope for that phase isn't
nailed down yet, so don't assume the deferred items above are its final
shape.

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
- **component** (`src/component.rs`): generic UI-concept token →
  semantic-role string mapping, e.g. `"button.background" = "surface_card"`.
  Deliberately *not* app-namespaced (not `waybar.button.background`) —
  originally planned that way, but changed before anything consumed it:
  (1) several themable apps share the same conceptual widgets (buttons,
  bars, menus, tooltips), so one shared token keeps a theme change
  cohesive across all of them, where a per-app name couldn't be reused;
  (2) it decouples this tier from the (not-yet-built) template engine's
  timeline and from which apps are actually themed — swapping waybar for
  another bar, or adding a new themable app, never touches this file,
  since the vocabulary never names an app. Resolves against the
  *resolved semantic* map (not `Primitives` directly) via
  `component::resolve`, reusing `auto:bg`/`auto:fg` handling for free
  instead of reimplementing it a layer up. Status roles
  (`success`/`warning`/`danger`/`info`) are intentionally not duplicated
  here since templates can reference those semantic roles directly; this
  tier only adds value for structural/container concepts (background/
  foreground/border on window/bar/button/menu/tooltip/selection) with no
  1:1 semantic equivalent. `Config::component` is an optional path
  override, same shape as `Config::semantic`. What the (not-yet-built)
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
     (`hue_lightness_bend`, see `HISTORY.md`), chroma capped by
     `stats.mean_c * chroma_clamp_factor` and tapered near the ramp's edges
     to reduce gamut clipping.
   - `neutral` ramp: hue/chroma blended between a flat baseline (weighted
     circular mean hue across all clusters, fixed `neutral_tint_chroma`)
     and the wallpaper's *vivid* accent (`pick_vivid_accent`), via
     `neutral_accent_influence` (0 = flat baseline exactly, 1 = fully
     accent-tinted). Default 0 — opt-in, since how much personality bg/fg
     should have is a judgment call, and turning it up too far reintroduces
     matugen's "everything is one hue" problem, just relocated to bg/fg.
     Built by `generate_neutral_ramp`, which (unlike the chromatic ramps)
     skips `chroma_taper` entirely, so the tint shows fully at every step
     including the ramp extremes (`background`/`ansi_color0` = step 950 for
     dark wallpapers) — see `HISTORY.md`.
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

## Templates (`a16p render`)

`src/render.rs` renders per-application config templates (minijinja,
Jinja2 syntax) with the pipeline's colors. `~/source/matugen-themes/`
(matugen's own template collection) was reviewed as a structural
reference — manifest shape, what fields real per-app templates need, the
post-hook idea — not as literal template source: its color names are
Material-You-specific (`primary`, `on_surface`, `error_container`, ...)
and don't map onto this project's semantic/component role names, so
templates need rewriting regardless of syntax. Default minijinja syntax
(`{% %}`/`{{ }}`) is used rather than matching matugen's custom `<* *>`
block delimiters, since there's no real porting win once variable names
already differ.

- **Manifest**: `[templates.<name>]` tables live directly in
  `config.toml` (`Config::templates: BTreeMap<String, TemplateEntry>`,
  `src/config.rs`) — no second manifest file. Field names
  (`input_path`/`output_path`/`post_hook`) deliberately match matugen's
  own, so per-app entries (and their reload commands) translate directly
  if referenced from matugen docs. Paths accept a leading `~`
  (`xdg::expand_tilde`).
- **Context**: `render::build_context` exposes `primitives`, `semantic`,
  `component` (the same three tiers as the JSON outputs) plus `image`
  (the wallpaper path) to every template. Semantic/primitive keys have no
  dots, so `{{ semantic.background.hex }}` works as plain attribute
  access; component token names contain dots (`button.background`), so
  they need bracket access: `{{ component["button.background"].hex }}`.
  Iterating a whole tier uses minijinja's documented `|dictsort` idiom —
  `{% for name, value in component|dictsort %}` — the equivalent of
  matugen's `<* for name, value in colors *>`.
- **`hex_stripped`**: matugen exposes hex-without-`#` as a plain
  attribute per color. Rather than a minijinja custom filter, `Swatch`
  (`src/palette_gen.rs`) just carries a `hex_stripped` field, computed
  once in `swatch_from_oklch`. Flows into `primitives.json`/
  `semantic.json`/`component.json` too, which is harmless.
- **`--dry-run`**: prints each template's rendered content and would-be
  output path to stdout, writes nothing, runs no hooks — the templating
  equivalent of `preview`'s file-write-free pipeline run, for fast
  template-authoring iteration.
- **`--run-hooks`**: `post_hook` (an arbitrary shell command) only runs
  when this flag is passed — writing files is the safe default, since
  hooks are shell commands sourced from a config file. A failing/missing
  hook prints to stderr but doesn't stop rendering of the remaining
  templates (files for every template are already written by the time
  hooks run).

## Known rough edges (expected iteration, not bugs to silently "fix")

None currently tracked — full narrative log of what's been found and fixed
lives in `HISTORY.md` (split out 2026-08-11 to keep this file lean); this
list is just pointers. Add new ones here as they turn up (with the full
story in `HISTORY.md`); don't silently "fix" something noted here without
calling it out, since it may be a deliberate v1 tradeoff rather than a bug.

- **Fixed:** `fallback_rotate`'s confidence damping was a flat, unexplained
  constant — see `HISTORY.md`.
- **Fixed:** naive per-channel gamut clamping instead of proper gamut
  mapping — see `HISTORY.md`.
- **Fixed:** hue-agnostic lightness curve made vivid yellow read as
  olive/brown — see `HISTORY.md`.
- **Fixed:** small vivid accents got crushed by weight-based selection —
  see `HISTORY.md`.
- **Fixed:** `primary`/preview accent was gray on the same wallpaper this
  whole fix was about — see `HISTORY.md`.
- **Fixed:** `match_hue` let a weak nearest-angle cluster outrank a more
  vivid farther one — see `HISTORY.md`.
- **Fixed:** `chroma_taper` crushed `neutral_accent_influence` specifically
  where it mattered most (bg/color0) — see `HISTORY.md`.

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
  `HISTORY.md`), hex, circular hue distance, pure-primary corner
  computation. Pure math, unit tested.
- `src/extract.rs` — image loading/downsampling, k-means, image stats
  (`mean_c` vs `chromatic_mean_c`, see `HISTORY.md`). Unit tested
  (cluster separation, weight normalization, chromatic_mean_c behavior).
- `src/palette_gen.rs` — hue matching/fallback (`match_hue` and
  `fallback_rotate` both pick by chroma among clusters clearing
  `min_weight`, not by weight or nearest angle; fallback also damps
  confidence by rotation distance, see `HISTORY.md`), ramp generation
  (neutral ramp skips `chroma_taper`, see `HISTORY.md`), primitives
  assembly. Unit tested (monotonic ramps, chroma cap, hue matching/fallback
  correctness including the vivid-vs-prevalent case and confidence damping,
  anchor spread).
- `src/semantic.rs` — default mapping, resolution logic. Unit tested
  (parses, resolves, `auto:bg`/`auto:fg` pick correctly, unknown ramp errors).
- `src/component.rs` — generic component-token → semantic-role mapping,
  resolution logic (a lookup against the resolved semantic map, no
  `ramp.step`/`auto:*` parsing needed since semantic already did that).
  Unit tested (parses, resolves, resolved value matches the semantic role
  it points at, unknown role errors).
- `src/config.rs` — `Config` (TOML-loadable, has `Default`), XDG-aware
  `load`, `annotated_default_toml` template, maps to `GenParams`.
  `TemplateEntry`/`Config::templates` hold the `[templates.*]` manifest
  consumed by `render.rs`.
- `src/xdg.rs` — XDG Base Directory config path resolution, plus
  `expand_tilde` for template input/output paths. Unit tested.
- `src/render.rs` — minijinja context building (`primitives`/`semantic`/
  `component`/`image`) and per-template rendering, consumed by
  `Command::Render`. Unit tested (attribute vs bracket access, `|dictsort`
  loop over a whole tier, `hex_stripped`, undefined-variable errors) —
  all against an in-memory context, no filesystem needed.
- `src/preview.rs` — `print_ansi16` (`background`/`accent`/`foreground` row,
  where accent = `resolved["primary"]`, then the raw ANSI16 swatch) and
  `print_terminal_mock` (prompt/`ls`/log-levels/diff/code-line mockup using
  the resolved bg/fg/ansi colors together) — isolated color blocks made it
  hard to judge fg-on-bg contrast and overall feel, the mockup is meant to
  approximate "does this look like a real terminal."
- `src/main.rs` — clap CLI (`generate`, `preview`, `config init`/`path`,
  `clusters` for raw k-means diagnostics, `render` for template
  rendering), wires the pipeline.

## Testing

`mise run test` (49 unit tests as of the templating slice, all
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
