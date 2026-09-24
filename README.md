# a16p-gen

Wallpaper-anchored ANSI16 palette generator, built to replace the "pick one
accent color, derive everything as shades of it" approach used by
[matugen](https://github.com/InioX/matugen) and the "spread across hues but
drift from the wallpaper's actual brightness/saturation" approach used by
[wallust](https://codeberg.org/explosion-mental/wallust).

## Why

ANSI16 terminal colors carry semantics: color0/7 are background/foreground,
1/9 are red (errors), 2/10 green (success), 3/11 yellow (warnings), and so on.
Deriving the whole palette from one picked accent collapses that into
near-monochrome shades. Spreading hues without anchoring to the image's own
lightness/chroma produces colors that look "off" against the wallpaper.

This tool clusters the source image in Oklab space, then for each of the six
ANSI hue slots (red/yellow/green/cyan/blue/magenta) snaps to the nearest
matching cluster hue — falling back to a hue-rotated prominent cluster when
the image genuinely has none of that hue — while clamping lightness/chroma to
what the image actually contains. See `CLAUDE.md` for the full algorithm and
architecture writeup.

## Status

v1: palette engine (primitive/semantic/component tiers) plus templating
(`a16p render`) — takes an image, outputs color tokens, and renders
per-app config templates from them. Still **no** dotfile installation
(symlink/copy management, installed-app discovery, template collection
management) beyond a template's own `post_hook`. Remaining v1 work is
polishing the palette generation algorithm itself against real
wallpapers; a v2 with a substantially larger feature set is planned
after that.

## Usage

```sh
# terminal mockup + raw ANSI16 swatch, no files written -- the fast iteration loop
mise run preview -- ./wallpaper.jpg

# full run: writes primitives.json + semantic.json + component.json, also prints the preview
mise run generate -- ./wallpaper.jpg -o ./out

# raw k-means clusters (weight/oklch/hex) + hue anchors -- for diagnosing why a slot matched or didn't
cargo run -- clusters ./wallpaper.jpg

# render every [templates.*] entry from config.toml with the generated colors
cargo run -- render ./wallpaper.jpg
cargo run -- render ./wallpaper.jpg --dry-run   # print to stdout, write nothing
cargo run -- render ./wallpaper.jpg --run-hooks # also run each template's post_hook
```

`preview`/`generate` print a small terminal mockup (prompt, `ls`, log
levels, a diff, a code line) using the resolved colors together, since
isolated swatch blocks make bg/fg contrast and overall feel hard to judge
in isolation — followed by a `background`/`accent`/`foreground` swatch row
and the raw ANSI16 swatch for precise before/after comparison across config
tweaks. "Accent" there is `primary` (see Output below) — the wallpaper's
vivid, defining color, not just its most common color.

Or without mise:

```sh
cargo run -- preview ./wallpaper.jpg
cargo run -- generate ./wallpaper.jpg -o ./out
```

### Config

Config is TOML, and every field is optional — set only what you're tuning,
the rest fall back to defaults. Resolution order:

1. `--config path/to/config.toml` if passed — must exist.
2. Otherwise `$XDG_CONFIG_HOME/a16p-gen/config.toml` (`~/.config/a16p-gen/config.toml`
   if `$XDG_CONFIG_HOME` is unset), if present.
3. Otherwise built-in defaults, silently.

```sh
mise run config-init          # write a fully-commented template to the XDG path
mise run config-init -- --force  # overwrite an existing one
mise run config-path          # print the resolved path
```

See `Config` in `src/config.rs` for all fields (cluster count `k`,
`hue_tolerance`, `min_cluster_weight`, `chroma_clamp_factor`,
`neutral_tint_chroma`, `neutral_accent_influence`, `max_dim`, `max_iters`,
`mode`, `dark_threshold`, and an optional `semantic` path to override the
default role→primitive mapping) — the generated template documents each one inline. Four are
most worth tuning per wallpaper style:

- `hue_tolerance` — how readily a slot direct-matches vs. falls back to
  hue-rotation.
- `chroma_clamp_factor` — how vivid/muted the result reads relative to the
  source image's own chromatic content.
- `min_cluster_weight` — how small a color region can be and still count as
  a real accent (vs. noise). Matters a lot for stylized/pixel-art wallpapers
  with small deliberate highlights against a large muted/dark background;
  too high and the highlight gets ignored in favor of the muted majority.
- `neutral_accent_influence` (0..1, default 0) — how much bg/fg get pulled
  toward the wallpaper's vivid accent color instead of staying flat gray.
  0 = flat (original behavior); higher = more matugen-like accent-tinted
  backgrounds. The effect is intentionally subtle right at `background`/
  `foreground` themselves (chroma is damped near the ramp's extremes to
  avoid ugly saturated near-black clipping) — check `surface_card`/
  `surface_border` in `semantic.json` for the more visible mid-ramp tint.

Dark vs light: `mode = "auto"` (default) calls a wallpaper dark when its
weighted mean lightness is below `dark_threshold` (0.55); `mode = "dark"`
or `"light"` forces one. Semantic roles that differ per mode are written
as pairs in the semantic mapping, e.g.
`surface_card = { dark = "neutral.900", light = "neutral.100" }`;
templates don't change.

`a16p clusters <image>` prints the raw k-means clusters (weight, Oklch,
hex) and the six hue-slot target angles — useful for seeing exactly why a
slot matched (or didn't) before reaching for these knobs blind.

## Output

- **`primitives.json`** — generated color ramps: `red`, `yellow`, `green`,
  `cyan`, `blue`, `magenta`, `neutral`, `accent`, `highlight`. Each ramp
  has steps `50..950` (Tailwind/PrimeNG-style), each step carrying `hex`,
  `rgb`, and `oklch`. `accent` is the single most _common_ cluster (what
  matugen would extract); `highlight` is the most _vivid_ cluster with real
  presence — for a mostly-dark wallpaper with a small saturated highlight,
  `accent` is often just a boring dark/gray color, `highlight` is the one
  that actually reads as "this wallpaper's color."
- **`semantic.json`** — resolved roles (`ansi_color0`..`ansi_color15`,
  `background`, `foreground`, `primary` (→ `highlight.500`), `success`,
  `warning`, `danger`, `info`, `surface_*`, `text_*`) as concrete swatches,
  per the default mapping in `src/semantic.rs::DEFAULT_SEMANTIC_TOML`.
- **`component.json`** — generic UI-concept tokens (e.g.
  `button.background`, `status.danger`, `terminal.color1`,
  `syntax.string`) resolved against `semantic.json`, per the default
  mapping in `src/component.rs`. App-agnostic by design — no
  `waybar.*`/`rofi.*` namespacing — so one theme change stays cohesive
  across every themed app. This is the only tier templates can use.

## Templates

`a16p render` renders per-app config files (minijinja/Jinja2 syntax) from
`component`/`image` context, driven by
`[templates.<name>]` entries in `config.toml`:

```toml
[templates.waybar]
input_path = "~/.config/a16p-gen/templates/waybar.css.jinja"
output_path = "~/.config/waybar/style.css"
post_hook = "killall -SIGUSR2 waybar"
```

Templates only see component tokens, accessed with brackets since their
names contain a literal dot: `{{ component["button.background"].hex }}`
(also `.hex_stripped`, `.rgb[0..2]`). `semantic`/`primitives` are not in
the template context — they're the upstream theming layer (dark/light
choice, ramp steps), and referencing them is a render error. `post_hook` only runs with `--run-hooks` passed; `--dry-run`
prints rendered output/paths without writing files or running hooks.

## Development

```sh
mise run build      # cargo build
mise run test        # cargo test
mise run lint         # cargo clippy -D warnings
mise run fmt           # cargo fmt
mise run ci              # fmt-check + lint + test
```

Tuning the algorithm itself (hue-match tolerance, chroma clamping, ramp
curve) is expected iteration — use `mise run preview` against real
wallpapers and eyeball the swatch; there's no automated "looks good" check
for this by nature.
