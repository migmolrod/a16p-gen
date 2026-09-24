use crate::config::TemplateEntry;
use crate::palette_gen::Swatch;
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

/// Templates only ever see the component tier. Primitives and semantic
/// roles are upstream theming decisions (ramp steps, dark/light picks);
/// keeping them out of the context means an app template can't bypass the
/// component tier -- referencing `semantic.*`/`primitives.*` fails to
/// render instead of silently coupling an app to a lower tier.
pub fn build_context(component: &BTreeMap<String, Swatch>, image: &Path) -> minijinja::Value {
    minijinja::context! {
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
    component: &BTreeMap<String, Swatch>,
    image: &Path,
) -> Result<Vec<RenderedTemplate>> {
    let env = make_environment();
    let ctx = build_context(component, image);

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

    fn sample_component() -> BTreeMap<String, Swatch> {
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
        crate::component::resolve(&component_mapping, &semantic).unwrap()
    }

    #[test]
    fn renders_component_token_by_bracket_access() {
        let component = sample_component();
        let env = make_environment();
        let ctx = build_context(&component, Path::new("/tmp/x.jpg"));
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
        let component = sample_component();
        let env = make_environment();
        let ctx = build_context(&component, Path::new("/tmp/x.jpg"));
        let out = render_one(
            &env,
            r#"{{ component["window.background"].hex_stripped }}"#,
            &ctx,
        )
        .unwrap();
        assert_eq!(out, component["window.background"].hex_stripped);
        assert!(!out.starts_with('#'));
    }

    #[test]
    fn dictsort_loop_visits_every_component_token() {
        let component = sample_component();
        let env = make_environment();
        let ctx = build_context(&component, Path::new("/tmp/x.jpg"));
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
    fn lower_tiers_are_not_exposed_to_templates() {
        let component = sample_component();
        let env = make_environment();
        let ctx = build_context(&component, Path::new("/tmp/x.jpg"));
        assert!(render_one(&env, "{{ semantic.background.hex }}", &ctx).is_err());
        assert!(render_one(&env, r#"{{ primitives.red["500"].hex }}"#, &ctx).is_err());
    }

    #[test]
    fn unknown_component_token_errors() {
        let component = sample_component();
        let env = make_environment();
        let ctx = build_context(&component, Path::new("/tmp/x.jpg"));
        assert!(render_one(&env, r#"{{ component["nope"].hex }}"#, &ctx).is_err());
    }
}
