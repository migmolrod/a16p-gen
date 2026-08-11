use crate::palette_gen::GenParams;
use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

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
    pub semantic: Option<String>,
    pub component: Option<String>,
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
            semantic: None,
            component: None,
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
# Lower = only very close hues match, more images fall back to hue
# rotation. Higher = looser matches, risks calling orange "red".
hue_tolerance = {hue_tolerance}

# Minimum cluster weight (fraction of image pixels, 0..1) required for a
# hue match to count. Filters real noise, not small deliberate accents --
# k-means already absorbs single-pixel/anti-aliasing noise into bigger
# clusters, so this can stay low without letting junk through. Raise it
# if a stray few-pixel cluster is hijacking a hue slot; lower it if a
# small accent region (a sprite highlight, a lantern glow) is being
# ignored in favor of a larger but duller match.
min_cluster_weight = {min_cluster_weight}

# Ramp chroma is capped at (chromatic_mean_c * this factor), where
# chromatic_mean_c is the weighted mean chroma of only the *chromatic*
# clusters (near-gray/black clusters don't count) -- so a mostly-dark
# wallpaper with a small vivid accent doesn't get its accent crushed just
# because most of the image is achromatic. Lower = safer/more muted
# colors closer to the wallpaper's actual saturation. Higher = more
# vivid, more likely to look "off" from the image -- this is the wallust
# failure mode this tool is meant to avoid.
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

# Optional path to a custom semantic role -> primitive mapping TOML file,
# overriding the built-in default (src/semantic.rs::DEFAULT_SEMANTIC_TOML).
# semantic = "/path/to/semantic.toml"

# Optional path to a custom component -> semantic role mapping TOML file,
# overriding the built-in default (src/component.rs::DEFAULT_COMPONENT_TOML).
# component = "/path/to/component.toml"
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
    }
}
