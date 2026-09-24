use crate::extract::ImageStats;
use crate::palette_gen::{Primitives, Swatch};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

/// Default role -> primitive mapping. This is where the ANSI16 semantic
/// contract (0/7 bg/fg, 1/9 red=error, 2/10 green=success, ...) becomes
/// explicit, editable data instead of an emergent property of the palette
/// algorithm. A role is either one `"ramp.step"` used in both modes, or a
/// `{ dark = ..., light = ... }` pair picked by the effective dark/light
/// mode (`Config::mode`, falling back to the image's detected lightness).
/// `auto:bg` / `auto:fg` are shorthand for the neutral-extreme pair
/// (`{ dark = "neutral.950", light = "neutral.50" }` and its inverse).
/// Chromatic 500s stay fixed across modes for now; light-mode contrast
/// tuning for them is expected iteration, not a settled decision.
pub const DEFAULT_SEMANTIC_TOML: &str = r#"
primary = "highlight.500"
primary_muted = { dark = "highlight.700", light = "highlight.300" }
success = "green.500"
warning = "yellow.500"
danger = "red.500"
info = "cyan.500"

# Low-contrast status tints (diff backgrounds and the like) and a
# stronger warning variant (changed-text-within-a-changed-line).
success_subtle = { dark = "green.900", light = "green.100" }
warning_subtle = { dark = "yellow.900", light = "yellow.100" }
danger_subtle = { dark = "red.900", light = "red.100" }
warning_emphasis = { dark = "yellow.700", light = "yellow.300" }
magenta_emphasis = "magenta.600"

surface_ground = "auto:bg"
surface_card = { dark = "neutral.900", light = "neutral.100" }
surface_raised = { dark = "neutral.800", light = "neutral.200" }
surface_border = { dark = "neutral.700", light = "neutral.300" }

text_color = "auto:fg"
# The neutral ramp is lopsided (500 reads ~8-10:1 on a dark bg but <2:1
# on a light one), so muted text needs a darker step in light mode.
text_muted_color = { dark = "neutral.500", light = "neutral.700" }

background = "auto:bg"
foreground = "auto:fg"

ansi_color0 = "auto:bg"
ansi_color1 = "red.500"
ansi_color2 = "green.500"
ansi_color3 = "yellow.500"
ansi_color4 = "blue.500"
ansi_color5 = "magenta.500"
ansi_color6 = "cyan.500"
# 7/8 mirror with 0/15 (which follow bg/fg via auto:*): 7 is a dimmed
# foreground, 8 a lifted background, in both modes.
ansi_color7 = { dark = "neutral.300", light = "neutral.700" }
ansi_color8 = { dark = "neutral.600", light = "neutral.400" }
ansi_color9 = "red.400"
ansi_color10 = "green.400"
ansi_color11 = "yellow.400"
ansi_color12 = "blue.400"
ansi_color13 = "magenta.400"
ansi_color14 = "cyan.400"
ansi_color15 = "auto:fg"
"#;

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(untagged)]
pub enum SemanticValue {
    Fixed(String),
    ByMode(ModePair),
}

#[derive(Deserialize, Debug, Clone, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ModePair {
    pub dark: String,
    pub light: String,
}

impl SemanticValue {
    fn for_mode(&self, is_dark: bool) -> &str {
        match self {
            SemanticValue::Fixed(v) => v,
            SemanticValue::ByMode(pair) if is_dark => &pair.dark,
            SemanticValue::ByMode(pair) => &pair.light,
        }
    }
}

pub type SemanticMapping = BTreeMap<String, SemanticValue>;

pub fn parse_mapping(toml_str: &str) -> Result<SemanticMapping> {
    // untagged-enum errors alone just say "did not match any variant".
    toml::from_str(toml_str).context(
        "invalid semantic mapping: each role must be \"ramp.step\", \"auto:bg\"/\"auto:fg\", \
         or {{ dark = \"ramp.step\", light = \"ramp.step\" }}",
    )
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
        .map(|(role, value)| {
            resolve_one(value.for_mode(stats.is_dark), primitives, stats)
                .with_context(|| format!("semantic role '{role}'"))
                .map(|s| (role.clone(), s))
        })
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
    fn primary_tracks_vivid_cluster_not_prevalent_one() {
        // sample_primitives' two clusters: 0.6 weight, low chroma (0.05);
        // 0.4 weight, higher chroma (0.18) at hue 29. primary should track
        // the more vivid one despite it being less prevalent -- this is
        // the whole point of primary mapping to `highlight`, not `accent`.
        let (primitives, _) = sample_primitives();
        let primary = primitives.highlight[&500].oklch;
        assert!((primary[2] - 29.0).abs() < 1.0);
    }

    #[test]
    fn auto_bg_fg_pick_opposite_neutral_extremes_when_dark() {
        let (primitives, stats) = sample_primitives();
        let bg = resolve_one("auto:bg", &primitives, &stats).unwrap();
        let fg = resolve_one("auto:fg", &primitives, &stats).unwrap();
        assert!(bg.oklch[0] < fg.oklch[0]);
    }

    #[test]
    fn mode_pair_picks_side_by_effective_mode() {
        let mapping =
            parse_mapping(r#"card = { dark = "neutral.900", light = "neutral.100" }"#).unwrap();
        let (primitives, mut stats) = sample_primitives();
        stats.is_dark = true;
        let dark = resolve(&mapping, &primitives, &stats).unwrap();
        assert_eq!(dark["card"], primitives.neutral[&900]);
        stats.is_dark = false;
        let light = resolve(&mapping, &primitives, &stats).unwrap();
        assert_eq!(light["card"], primitives.neutral[&100]);
    }

    #[test]
    fn fixed_value_ignores_mode() {
        let mapping = parse_mapping(r#"primary = "highlight.500""#).unwrap();
        let (primitives, mut stats) = sample_primitives();
        stats.is_dark = false;
        let light = resolve(&mapping, &primitives, &stats).unwrap();
        assert_eq!(light["primary"], primitives.highlight[&500]);
    }

    #[test]
    fn mode_pair_rejects_typoed_keys() {
        assert!(
            parse_mapping(r#"card = { dark = "neutral.900", ligth = "neutral.100" }"#).is_err()
        );
    }

    #[test]
    fn unknown_ramp_errors() {
        let (primitives, stats) = sample_primitives();
        assert!(resolve_one("nope.500", &primitives, &stats).is_err());
    }
}
