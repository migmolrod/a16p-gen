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

v1: palette engine only. Takes an image, outputs color tokens. Does **not**
yet render templates or install dotfiles — that's deferred until the
algorithm quality holds up against real wallpapers.

## Usage

```sh
# terminal swatch only, no files written -- the fast iteration loop
mise run preview -- ./wallpaper.jpg

# full run: writes primitives.json + semantic.json, also prints swatch
mise run generate -- ./wallpaper.jpg -o ./out
```

Or without mise:

```sh
cargo run -- preview ./wallpaper.jpg
cargo run -- generate ./wallpaper.jpg -o ./out
```

### Config

Both commands take `--config path/to/config.toml`. See `Config` in
`src/config.rs` for all fields (cluster count `k`, `hue_tolerance`,
`chroma_clamp_factor`, `neutral_tint_chroma`, `max_dim`, `max_iters`, and an
optional `semantic` path to override the default role→primitive mapping).
Without `--config`, built-in defaults are used.

## Output

- **`primitives.json`** — generated color ramps: `red`, `yellow`, `green`,
  `cyan`, `blue`, `magenta`, `neutral`, `accent`. Each ramp has steps
  `50..950` (Tailwind/PrimeNG-style), each step carrying `hex`, `rgb`,
  and `oklch`.
- **`semantic.json`** — resolved roles (`ansi_color0`..`ansi_color15`,
  `background`, `foreground`, `primary`, `success`, `warning`, `danger`,
  `info`, `surface_*`, `text_*`) as concrete swatches, per the default
  mapping in `src/semantic.rs::DEFAULT_SEMANTIC_TOML`.

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
