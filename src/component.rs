use crate::palette_gen::Swatch;
use anyhow::Result;
use std::collections::BTreeMap;

/// Default component -> semantic role mapping. Component tokens are
/// generic UI-concept names (`button.background`, not
/// `waybar.button.background`) so multiple themable apps can share the
/// same token -- retuning one component here updates every app template
/// that consumes it, and adding/swapping a themable app never requires
/// touching this tier. This is the *only* tier templates can see
/// (`render::build_context` doesn't expose `semantic`/`primitives`), so it
/// has to cover everything an app needs -- including status colors, the
/// terminal ANSI16 palette and syntax highlighting, even where a token is
/// a straight 1:1 alias of a semantic role today. The indirection is the
/// point: e.g. `syntax.string` and `status.success` are both green now,
/// but can be retuned independently without touching any template.
// Keys are quoted so TOML treats each as one flat string key (matching
// `ComponentMapping`'s `BTreeMap<String, String>`) instead of parsing the
// dot as a nested-table path, which would produce a map of maps.
pub const DEFAULT_COMPONENT_TOML: &str = r#"
"window.background" = "background"
"window.foreground" = "foreground"
"window.shadow" = "background"

"text.default" = "text_color"
"text.muted" = "text_muted_color"

"surface.ground" = "surface_ground"
"surface.card" = "surface_card"
"surface.raised" = "surface_raised"
"surface.hover" = "surface_raised"

"accent.default" = "primary"
"accent.muted" = "primary_muted"
"accent.secondary" = "info"
"accent.contrast" = "background"

"link.default" = "info"

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
"selection.inactive_background" = "primary_muted"

# `status.contrast` is text/icons drawn on top of a status-colored fill.
"status.success" = "success"
"status.warning" = "warning"
"status.danger" = "danger"
"status.info" = "info"
"status.hint" = "ansi_color4"
"status.contrast" = "background"

"diff.added" = "success"
"diff.changed" = "warning"
"diff.removed" = "danger"
"diff.added_background" = "success_subtle"
"diff.changed_background" = "warning_subtle"
"diff.removed_background" = "danger_subtle"
"diff.text_background" = "warning_emphasis"

"editor.background" = "background"
"editor.foreground" = "foreground"
"editor.cursor" = "foreground"
"editor.cursor_text" = "background"
"editor.cursor_line" = "surface_raised"
"editor.line_number" = "text_muted_color"
"editor.line_number_active" = "primary"
"editor.guide" = "surface_border"
"editor.match_background" = "surface_border"
"editor.search_background" = "primary"
"editor.search_foreground" = "background"
"editor.reference_background" = "surface_raised"

# Syntax colors pick from the ANSI hue slots rather than the status roles:
# strings are green because of hue, not because they mean "success".
"syntax.comment" = "text_muted_color"
"syntax.punctuation" = "text_muted_color"
"syntax.operator" = "text_color"
"syntax.variable" = "foreground"
"syntax.variable_builtin" = "ansi_color1"
"syntax.identifier" = "ansi_color1"
"syntax.tag" = "ansi_color1"
"syntax.string" = "ansi_color2"
"syntax.type" = "ansi_color3"
"syntax.tag_attribute" = "ansi_color3"
"syntax.function" = "ansi_color4"
"syntax.constant" = "ansi_color5"
"syntax.keyword" = "ansi_color5"
"syntax.property" = "ansi_color6"
"syntax.preproc" = "ansi_color6"
"syntax.special" = "ansi_color6"
"syntax.escape" = "ansi_color6"

"terminal.background" = "background"
"terminal.foreground" = "foreground"
"terminal.cursor" = "foreground"
"terminal.cursor_text" = "background"
"terminal.url" = "info"
"terminal.color0" = "ansi_color0"
"terminal.color1" = "ansi_color1"
"terminal.color2" = "ansi_color2"
"terminal.color3" = "ansi_color3"
"terminal.color4" = "ansi_color4"
"terminal.color5" = "ansi_color5"
"terminal.color6" = "ansi_color6"
"terminal.color7" = "ansi_color7"
"terminal.color8" = "ansi_color8"
"terminal.color9" = "ansi_color9"
"terminal.color10" = "ansi_color10"
"terminal.color11" = "ansi_color11"
"terminal.color12" = "ansi_color12"
"terminal.color13" = "ansi_color13"
"terminal.color14" = "ansi_color14"
"terminal.color15" = "ansi_color15"

# Decorative per-hue picks for UIs that color things by hue without any
# status meaning (bar modules, statusline segments).
"hue.red" = "ansi_color1"
"hue.green" = "ansi_color2"
"hue.yellow" = "ansi_color3"
"hue.blue" = "ansi_color4"
"hue.magenta" = "ansi_color5"
"hue.cyan" = "ansi_color6"
"hue.bright_yellow" = "ansi_color11"
"hue.dark_magenta" = "magenta_emphasis"
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
