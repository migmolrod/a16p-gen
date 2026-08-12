use crate::config::TemplateEntry;
use crate::palette_gen::{Primitives, Swatch};
use anyhow::{Context, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub struct RenderedTemplate {
    pub name: String,
    pub output_path: PathBuf,
    pub content: String,
    pub post_hook: Option<String>,
}

/// One place to register filters, should any be needed later --
/// `hex_stripped` (matugen's plain-attribute equivalent) is a `Swatch`
/// field instead, so none are registered yet.
fn make_environment() -> minijinja::Environment<'static> {
    minijinja::Environment::new()
}

pub fn build_context(
    primitives: &Primitives,
    semantic: &BTreeMap<String, Swatch>,
    component: &BTreeMap<String, Swatch>,
    image: &Path,
) -> minijinja::Value {
    minijinja::context! {
        primitives => primitives,
        semantic => semantic,
        component => component,
        image => image.display().to_string(),
    }
}

pub fn render_one(
    env: &minijinja::Environment,
    source: &str,
    ctx: &minijinja::Value,
) -> Result<String> {
    Ok(env.render_str(source, ctx)?)
}

pub fn render_all(
    templates: &BTreeMap<String, TemplateEntry>,
    primitives: &Primitives,
    semantic: &BTreeMap<String, Swatch>,
    component: &BTreeMap<String, Swatch>,
    image: &Path,
) -> Result<Vec<RenderedTemplate>> {
    let env = make_environment();
    let ctx = build_context(primitives, semantic, component, image);

    templates
        .iter()
        .map(|(name, entry)| {
            let input_path = crate::xdg::expand_tilde(&entry.input_path);
            let source = std::fs::read_to_string(&input_path).with_context(|| {
                format!(
                    "failed to read template '{name}' at {}",
                    input_path.display()
                )
            })?;
            let content = render_one(&env, &source, &ctx)
                .with_context(|| format!("failed to render template '{name}'"))?;
            Ok(RenderedTemplate {
                name: name.clone(),
                output_path: crate::xdg::expand_tilde(&entry.output_path),
                content,
                post_hook: entry.post_hook.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::{Cluster, ImageStats};
    use crate::palette_gen::GenParams;

    fn sample_tiers() -> (Primitives, BTreeMap<String, Swatch>, BTreeMap<String, Swatch>) {
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
        let semantic_mapping =
            crate::semantic::parse_mapping(crate::semantic::DEFAULT_SEMANTIC_TOML).unwrap();
        let semantic = crate::semantic::resolve(&semantic_mapping, &primitives, &stats).unwrap();
        let component_mapping =
            crate::component::parse_mapping(crate::component::DEFAULT_COMPONENT_TOML).unwrap();
        let component = crate::component::resolve(&component_mapping, &semantic).unwrap();
        (primitives, semantic, component)
    }

    #[test]
    fn renders_semantic_role_by_attribute_access() {
        let (primitives, semantic, component) = sample_tiers();
        let env = make_environment();
        let ctx = build_context(&primitives, &semantic, &component, Path::new("/tmp/x.jpg"));
        let out = render_one(&env, "bg = {{ semantic.background.hex }}", &ctx).unwrap();
        assert_eq!(out, format!("bg = {}", semantic["background"].hex));
    }

    #[test]
    fn renders_component_token_by_bracket_access() {
        let (primitives, semantic, component) = sample_tiers();
        let env = make_environment();
        let ctx = build_context(&primitives, &semantic, &component, Path::new("/tmp/x.jpg"));
        let out = render_one(
            &env,
            r#"btn = {{ component["button.background"].hex }}"#,
            &ctx,
        )
        .unwrap();
        assert_eq!(out, format!("btn = {}", component["button.background"].hex));
    }

    #[test]
    fn renders_hex_stripped_field() {
        let (primitives, semantic, component) = sample_tiers();
        let env = make_environment();
        let ctx = build_context(&primitives, &semantic, &component, Path::new("/tmp/x.jpg"));
        let out = render_one(&env, "{{ semantic.background.hex_stripped }}", &ctx).unwrap();
        assert_eq!(out, semantic["background"].hex_stripped);
        assert!(!out.starts_with('#'));
    }

    #[test]
    fn dictsort_loop_visits_every_component_token() {
        let (primitives, semantic, component) = sample_tiers();
        let env = make_environment();
        let ctx = build_context(&primitives, &semantic, &component, Path::new("/tmp/x.jpg"));
        let out = render_one(
            &env,
            "{% for name, value in component|dictsort %}{{ name }};{% endfor %}",
            &ctx,
        )
        .unwrap();
        for name in component.keys() {
            assert!(out.contains(&format!("{name};")));
        }
    }

    #[test]
    fn unknown_template_variable_errors() {
        let (primitives, semantic, component) = sample_tiers();
        let env = make_environment();
        let ctx = build_context(&primitives, &semantic, &component, Path::new("/tmp/x.jpg"));
        assert!(render_one(&env, "{{ semantic.nope.hex }}", &ctx).is_err());
    }
}
