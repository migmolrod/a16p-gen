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
    pub semantic: Option<String>,
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
            semantic: None,
        }
    }
}

impl Config {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path {
            Some(p) => {
                let text = std::fs::read_to_string(p)
                    .with_context(|| format!("failed to read config {}", p.display()))?;
                Ok(toml::from_str(&text)
                    .with_context(|| format!("failed to parse config {}", p.display()))?)
            }
            None => Ok(Config::default()),
        }
    }

    pub fn gen_params(&self) -> GenParams {
        GenParams {
            hue_tolerance: self.hue_tolerance,
            min_cluster_weight: self.min_cluster_weight,
            chroma_clamp_factor: self.chroma_clamp_factor,
            neutral_tint_chroma: self.neutral_tint_chroma,
        }
    }
}
