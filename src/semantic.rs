use crate::extract::ImageStats;
use crate::palette_gen::{Primitives, Swatch};
use anyhow::{Result, bail};
use std::collections::BTreeMap;

/// Default role -> primitive mapping. This is where the ANSI16 semantic
/// contract (0/7 bg/fg, 1/9 red=error, 2/10 green=success, ...) becomes
/// explicit, editable data instead of an emergent property of the palette
/// algorithm. `auto:bg` / `auto:fg` resolve to whichever neutral extreme
/// matches the detected dark/light mode of the source image.
pub const DEFAULT_SEMANTIC_TOML: &str = r#"
primary = "accent.500"
success = "green.500"
warning = "yellow.500"
danger = "red.500"
info = "cyan.500"

surface_ground = "auto:bg"
surface_card = "neutral.900"
surface_border = "neutral.700"

text_color = "auto:fg"
text_muted_color = "neutral.500"

background = "auto:bg"
foreground = "auto:fg"

ansi_color0 = "auto:bg"
ansi_color1 = "red.500"
ansi_color2 = "green.500"
ansi_color3 = "yellow.500"
ansi_color4 = "blue.500"
ansi_color5 = "magenta.500"
ansi_color6 = "cyan.500"
ansi_color7 = "neutral.300"
ansi_color8 = "neutral.600"
ansi_color9 = "red.400"
ansi_color10 = "green.400"
ansi_color11 = "yellow.400"
ansi_color12 = "blue.400"
ansi_color13 = "magenta.400"
ansi_color14 = "cyan.400"
ansi_color15 = "auto:fg"
"#;

pub type SemanticMapping = BTreeMap<String, String>;

pub fn parse_mapping(toml_str: &str) -> Result<SemanticMapping> {
    Ok(toml::from_str(toml_str)?)
}

fn resolve_one(value: &str, primitives: &Primitives, stats: &ImageStats) -> Result<Swatch> {
    if value == "auto:bg" {
        let step = if stats.is_dark { 950 } else { 50 };
        return Ok(primitives.neutral[&step].clone());
    }
    if value == "auto:fg" {
        let step = if stats.is_dark { 50 } else { 950 };
        return Ok(primitives.neutral[&step].clone());
    }
    let Some((ramp_name, step_str)) = value.split_once('.') else {
        bail!("invalid semantic value '{value}', expected 'ramp.step' or 'auto:bg'/'auto:fg'");
    };
    let ramp = primitives.get_ramp(ramp_name).ok_or_else(|| {
        anyhow::anyhow!("unknown primitive ramp '{ramp_name}' in value '{value}'")
    })?;
    let step: u16 = step_str
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid ramp step '{step_str}' in value '{value}'"))?;
    ramp.get(&step)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("ramp '{ramp_name}' has no step {step} (value '{value}')"))
}

pub fn resolve(
    mapping: &SemanticMapping,
    primitives: &Primitives,
    stats: &ImageStats,
) -> Result<BTreeMap<String, Swatch>> {
    mapping
        .iter()
        .map(|(role, value)| resolve_one(value, primitives, stats).map(|s| (role.clone(), s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{Cluster, ImageStats};
    use crate::palette_gen::{GenParams, build_primitives};

    fn sample_primitives() -> (Primitives, ImageStats) {
        let clusters = vec![
            Cluster {
                oklab: crate::color::oklch_to_oklab([0.2, 0.05, 250.0]),
                weight: 0.6,
            },
            Cluster {
                oklab: crate::color::oklch_to_oklab([0.6, 0.18, 29.0]),
                weight: 0.4,
            },
        ];
        let stats = ImageStats {
            mean_l: 0.36,
            mean_c: 0.1,
            chromatic_mean_c: 0.1,
            is_dark: true,
        };
        (
            build_primitives(&clusters, &stats, &GenParams::default()),
            stats,
        )
    }

    #[test]
    fn default_mapping_parses_and_resolves() {
        let mapping = parse_mapping(DEFAULT_SEMANTIC_TOML).unwrap();
        let (primitives, stats) = sample_primitives();
        let resolved = resolve(&mapping, &primitives, &stats).unwrap();
        assert_eq!(resolved.len(), mapping.len());
        assert!(resolved.contains_key("ansi_color1"));
        assert!(resolved.contains_key("background"));
    }

    #[test]
    fn auto_bg_fg_pick_opposite_neutral_extremes_when_dark() {
        let (primitives, stats) = sample_primitives();
        let bg = resolve_one("auto:bg", &primitives, &stats).unwrap();
        let fg = resolve_one("auto:fg", &primitives, &stats).unwrap();
        assert!(bg.oklch[0] < fg.oklch[0]);
    }

    #[test]
    fn unknown_ramp_errors() {
        let (primitives, stats) = sample_primitives();
        assert!(resolve_one("nope.500", &primitives, &stats).is_err());
    }
}
