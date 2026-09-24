use crate::palette_gen::GenParams;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// One `[templates.<name>]` entry: a template to render and where to
/// write it. Field names deliberately match matugen's own manifest
/// shape (`input_path`/`output_path`/`post_hook`) -- see
/// `~/source/matugen-themes/` for prior art on what real per-app entries
/// look like. Paths accept a leading `~` (`src/xdg.rs::expand_tilde`).
#[derive(Deserialize, Clone)]
pub struct TemplateEntry {
    pub input_path: String,
    pub output_path: String,
    #[serde(default)]
    pub post_hook: Option<String>,
}

/// Dark/light theme selection. `Auto` uses the wallpaper's detected
/// lightness (`ImageStats::is_dark`, compared against `dark_threshold`);
/// `Dark`/`Light` force a mode regardless of the image.
#[derive(Deserialize, Clone, Copy, Default, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Auto,
    Dark,
    Light,
}

impl ThemeMode {
    pub fn is_dark(self, detected: bool) -> bool {
        match self {
            ThemeMode::Auto => detected,
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Auto => "auto",
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub k: usize,
    pub max_iters: usize,
    pub max_dim: u32,
    pub hue_tolerance: f32,
    pub min_cluster_weight: f32,
    pub chroma_clamp_factor: f32,
    pub neutral_tint_chroma: f32,
    pub neutral_accent_influence: f32,
    pub hue_lightness_bend: f32,
    pub vibrancy_coherence: f32,
    pub fallback_vibrancy: f32,
    pub hue_shift_limit: f32,
    pub mode: ThemeMode,
    pub dark_threshold: f32,
    pub semantic: Option<String>,
    pub component: Option<String>,
    pub templates: BTreeMap<String, TemplateEntry>,
}

impl Default for Config {
    fn default() -> Self {
        let defaults = GenParams::default();
        Self {
            k: 16,
            max_iters: 20,
            max_dim: 200,
            hue_tolerance: defaults.hue_tolerance,
            min_cluster_weight: defaults.min_cluster_weight,
            chroma_clamp_factor: defaults.chroma_clamp_factor,
            neutral_tint_chroma: defaults.neutral_tint_chroma,
            neutral_accent_influence: defaults.neutral_accent_influence,
            hue_lightness_bend: defaults.hue_lightness_bend,
            vibrancy_coherence: defaults.vibrancy_coherence,
            fallback_vibrancy: defaults.fallback_vibrancy,
            hue_shift_limit: defaults.hue_shift_limit,
            mode: ThemeMode::Auto,
            dark_threshold: crate::extract::DEFAULT_DARK_THRESHOLD,
            semantic: None,
            component: None,
            templates: BTreeMap::new(),
        }
    }
}

fn parse_file(p: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(p)
        .with_context(|| format!("failed to read config {}", p.display()))?;
    toml::from_str(&text).with_context(|| format!("failed to parse config {}", p.display()))
}

impl Config {
    /// `path` (from `--config`) always wins and must exist. Otherwise, the
    /// XDG config path is used if present; if neither exists, built-in
    /// defaults apply silently -- a config file is optional, not required.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(p) => parse_file(p),
            None => {
                let default_path = crate::xdg::default_config_path();
                if default_path.exists() {
                    parse_file(&default_path)
                } else {
                    Ok(Config::default())
                }
            }
        }
    }

    pub fn gen_params(&self) -> GenParams {
        GenParams {
            hue_tolerance: self.hue_tolerance,
            min_cluster_weight: self.min_cluster_weight,
            chroma_clamp_factor: self.chroma_clamp_factor,
            neutral_tint_chroma: self.neutral_tint_chroma,
            neutral_accent_influence: self.neutral_accent_influence,
            hue_lightness_bend: self.hue_lightness_bend,
            vibrancy_coherence: self.vibrancy_coherence,
            fallback_vibrancy: self.fallback_vibrancy,
            hue_shift_limit: self.hue_shift_limit,
        }
    }

    /// Fully-commented TOML template written by `a16p config init`. Values
    /// are interpolated from `Config::default()` so the template can't
    /// silently drift out of sync with the actual defaults.
    pub fn annotated_default_toml() -> String {
        let d = Config::default();
        format!(
            r#"# a16p-gen config
# All fields are optional -- anything omitted keeps its default value.

# Number of k-means clusters extracted from the image in Oklab space.
# More clusters = finer color detail but slower, noisier hue matching.
k = {k}

# Max Lloyd's-algorithm iterations for k-means convergence.
max_iters = {max_iters}

# Longest-side pixel size the image is downsampled to before clustering.
# Bigger = more accurate sampling, slower.
max_dim = {max_dim}

# Max degrees a cluster's hue may differ from a target ANSI hue (red/
# yellow/green/cyan/blue/magenta) and still count as a direct match.
# Lower = only very close hues match, more slots fall back to their pure
# hue. Higher = more slots pick up a wallpaper color; how far their hue
# then moves is capped by hue_shift_limit, so orange can't become "red".
hue_tolerance = {hue_tolerance}

# Minimum cluster weight (fraction of image pixels, 0..1) required for a
# hue match to count. Filters real noise, not small deliberate accents --
# k-means already absorbs single-pixel/anti-aliasing noise into bigger
# clusters, so this can stay low without letting junk through. Raise it
# if a stray few-pixel cluster is hijacking a hue slot; lower it if a
# small accent region (a sprite highlight, a lantern glow) is being
# ignored in favor of a larger but duller match.
min_cluster_weight = {min_cluster_weight}

# Absolute chroma cap for the neutral ramp's accent tint (see
# neutral_accent_influence) and the matugen-parity `accent` ramp: capped at
# (chromatic_mean_c * this factor), where chromatic_mean_c is the weighted
# mean chroma of only the *chromatic* clusters (near-gray/black clusters
# don't count). Does NOT affect the red/yellow/.../magenta ramps or the
# primary/highlight ramp -- their vividness follows the primary's, relative
# to each hue's own gamut (see vibrancy_coherence).
chroma_clamp_factor = {chroma_clamp_factor}

# Chroma of the neutral/background ramp's baseline hue tint (used at
# neutral_accent_influence=0, and as the floor even above that).
# 0 = pure gray backgrounds; higher = more visibly tinted by the wallpaper.
neutral_tint_chroma = {neutral_tint_chroma}

# How much bg/fg should be pulled toward the wallpaper's vivid accent
# color (most saturated cluster with real presence, not just the most
# common one), from 0 (flat gray tint, ignores the accent entirely) to 1
# (bg/fg hue and chroma track the accent directly). This is a judgment
# call, not something to default aggressively: too high and you're back to
# matugen's "everything is one hue" problem, just relocated to bg/fg
# instead of the ANSI colors. Start around 0.3-0.5 and adjust by eye.
neutral_accent_influence = {neutral_accent_influence}

# Every hue reaches its highest possible chroma at a different lightness
# (pure yellow peaks near L=0.97, pure blue near L=0.45) -- but each ramp's
# step 500 was always forced to the same L≈0.53 regardless of hue, which is
# why mid-ramp yellow reads as muddy olive/brown instead of vivid. This
# bends step 500 toward the hue's actual peak-chroma lightness: 0 = old
# behavior (every hue forced through the same curve), 1 = step 500 lands
# exactly on the hue's peak (ramps across hues then span visibly different
# lightness ranges). Unlike neutral_accent_influence this isn't a style
# choice, so it defaults on.
hue_lightness_bend = {hue_lightness_bend}

# The red/yellow/green/cyan/blue/magenta ramps are generated at the same
# vibrancy as the primary color -- measured relative to how much chroma
# each hue can reach at each lightness, so a neon green primary gives a
# red that's equally neon *for a red*. Slots with no matching wallpaper
# color always take the primary's vibrancy. For slots that DO match a
# wallpaper color, this blends that color's own vibrancy toward the
# primary's: 1 = every slot exactly as vivid as the primary (most
# cohesive), 0 = matched colors keep the wallpaper's own saturation (a muted
# blue stays muted next to a vivid primary).
vibrancy_coherence = {vibrancy_coherence}

# Vibrancy of hues with NO matching wallpaper color (see hue_tolerance), as
# a fraction of the primary's: 1 = as vivid as the primary (can make an
# absent hue, like magenta on an orange wallpaper, the loudest color in the
# palette), lower = absent hues step back. Never drops below a floor that
# keeps the ANSI hues distinguishable from each other.
fallback_vibrancy = {fallback_vibrancy}

# How far a slot that matched a wallpaper color may move its hue toward
# that color, as a fraction of the distance to the neighboring ANSI hue
# (red<->yellow, yellow<->green, ...). 0 = every slot stays on its pure
# hue (only vibrancy comes from the wallpaper); 0.5 = up to halfway to the
# neighbor; 1 = could reach it. Keeps yellow from turning green (or red
# orange) when hue_tolerance is wide; hue_tolerance decides WHETHER a slot
# matches, this decides how far its hue follows.
hue_shift_limit = {hue_shift_limit}

# Dark/light theme: "auto" (detect from the wallpaper), "dark" or "light".
# Selects which side of each {{ dark = ..., light = ... }} semantic role
# is used, and which neutral extreme auto:bg/auto:fg resolve to.
# Templates don't change -- they only see component tokens.
mode = "{mode}"

# In "auto" mode, a wallpaper whose weighted mean lightness (Oklab L,
# 0..1, printed by 'a16p clusters' as mean_l) is below this counts as
# dark. Raise it to call more borderline wallpapers dark, lower it to
# call more of them light.
dark_threshold = {dark_threshold}

# Optional path to a custom semantic role -> primitive mapping TOML file,
# overriding the built-in default (src/semantic.rs::DEFAULT_SEMANTIC_TOML).
# semantic = "/path/to/semantic.toml"

# Optional path to a custom component -> semantic role mapping TOML file,
# overriding the built-in default (src/component.rs::DEFAULT_COMPONENT_TOML).
# component = "/path/to/component.toml"

# Per-application render templates ('a16p render'). Each entry maps a name
# to a template's input path (minijinja/Jinja2 syntax -- only component
# tokens are available, as `component["<token>"].hex` since token names
# contain dots) and where the
# rendered result should be written. `post_hook` (optional) is a shell
# command run after writing that template's output, only when
# `a16p render --run-hooks` is passed. Paths accept a leading `~` for $HOME.
#
# [templates.waybar]
# input_path = "~/.config/a16p-gen/templates/waybar-colors.css"
# output_path = "~/.config/waybar/colors.css"
# post_hook = "pkill -SIGUSR2 waybar"
"#,
            k = d.k,
            max_iters = d.max_iters,
            max_dim = d.max_dim,
            hue_tolerance = d.hue_tolerance,
            min_cluster_weight = d.min_cluster_weight,
            chroma_clamp_factor = d.chroma_clamp_factor,
            neutral_tint_chroma = d.neutral_tint_chroma,
            neutral_accent_influence = d.neutral_accent_influence,
            hue_lightness_bend = d.hue_lightness_bend,
            vibrancy_coherence = d.vibrancy_coherence,
            fallback_vibrancy = d.fallback_vibrancy,
            hue_shift_limit = d.hue_shift_limit,
            mode = d.mode.as_str(),
            dark_threshold = d.dark_threshold,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotated_default_toml_parses_back_to_defaults() {
        let text = Config::annotated_default_toml();
        let parsed: Config = toml::from_str(&text).unwrap();
        assert_eq!(parsed.k, Config::default().k);
        assert_eq!(parsed.hue_tolerance, Config::default().hue_tolerance);
        assert_eq!(
            parsed.chroma_clamp_factor,
            Config::default().chroma_clamp_factor
        );
        assert_eq!(
            parsed.vibrancy_coherence,
            Config::default().vibrancy_coherence
        );
        assert_eq!(
            parsed.fallback_vibrancy,
            Config::default().fallback_vibrancy
        );
        assert_eq!(parsed.hue_shift_limit, Config::default().hue_shift_limit);
    }
}
