use crate::palette_gen::Swatch;
use anyhow::Result;
use std::collections::BTreeMap;

/// Default component -> semantic role mapping. Component tokens are
/// generic UI-concept names (`button.background`, not
/// `waybar.button.background`) so multiple themable apps can share the
/// same token -- retuning one component here updates every app template
/// that consumes it, and adding/swapping a themable app never requires
/// touching this tier. Status roles (`success`/`warning`/`danger`/`info`)
/// are deliberately not duplicated here since templates can already
/// reference those semantic roles directly; this tier only adds value for
/// structural/container concepts without a 1:1 semantic equivalent.
// Keys are quoted so TOML treats each as one flat string key (matching
// `ComponentMapping`'s `BTreeMap<String, String>`) instead of parsing the
// dot as a nested-table path, which would produce a map of maps.
pub const DEFAULT_COMPONENT_TOML: &str = r#"
"window.background" = "background"
"window.foreground" = "foreground"

"bar.background" = "surface_ground"
"bar.foreground" = "text_color"
"bar.border" = "surface_border"

"button.background" = "surface_card"
"button.foreground" = "text_color"
"button.border" = "surface_border"
"button.active_background" = "primary"
"button.active_foreground" = "background"

"menu.background" = "surface_card"
"menu.foreground" = "text_color"
"menu.border" = "surface_border"
"menu.selected_background" = "primary"
"menu.selected_foreground" = "background"

"tooltip.background" = "surface_card"
"tooltip.foreground" = "text_color"
"tooltip.border" = "surface_border"

"border.default" = "surface_border"
"border.focus" = "primary"

"selection.background" = "primary"
"selection.foreground" = "background"
"#;

pub type ComponentMapping = BTreeMap<String, String>;

pub fn parse_mapping(toml_str: &str) -> Result<ComponentMapping> {
    Ok(toml::from_str(toml_str)?)
}

fn resolve_one(value: &str, resolved_semantic: &BTreeMap<String, Swatch>) -> Result<Swatch> {
    resolved_semantic
        .get(value)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown semantic role '{value}'"))
}

pub fn resolve(
    mapping: &ComponentMapping,
    resolved_semantic: &BTreeMap<String, Swatch>,
) -> Result<BTreeMap<String, Swatch>> {
    mapping
        .iter()
        .map(|(token, value)| resolve_one(value, resolved_semantic).map(|s| (token.clone(), s)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{Cluster, ImageStats};
    use crate::palette_gen::GenParams;

    fn sample_resolved_semantic() -> BTreeMap<String, Swatch> {
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
        let primitives =
            crate::palette_gen::build_primitives(&clusters, &stats, &GenParams::default());
        let mapping =
            crate::semantic::parse_mapping(crate::semantic::DEFAULT_SEMANTIC_TOML).unwrap();
        crate::semantic::resolve(&mapping, &primitives, &stats).unwrap()
    }

    #[test]
    fn default_mapping_parses_and_resolves() {
        let mapping = parse_mapping(DEFAULT_COMPONENT_TOML).unwrap();
        let resolved_semantic = sample_resolved_semantic();
        let resolved = resolve(&mapping, &resolved_semantic).unwrap();
        assert_eq!(resolved.len(), mapping.len());
        assert!(resolved.contains_key("button.background"));
    }

    #[test]
    fn component_matches_the_semantic_role_it_points_at() {
        let mapping = parse_mapping(DEFAULT_COMPONENT_TOML).unwrap();
        let resolved_semantic = sample_resolved_semantic();
        let resolved = resolve(&mapping, &resolved_semantic).unwrap();
        assert_eq!(
            resolved["button.background"],
            resolved_semantic["surface_card"]
        );
    }

    #[test]
    fn unknown_semantic_role_errors() {
        let resolved_semantic = sample_resolved_semantic();
        assert!(resolve_one("nope", &resolved_semantic).is_err());
    }
}
