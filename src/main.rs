mod color;
mod component;
mod config;
mod extract;
mod palette_gen;
mod preview;
mod semantic;
mod xdg;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "a16p", about = "Wallpaper-anchored ANSI16 palette generator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the full pipeline and write primitives.json + semantic.json + component.json
    Generate {
        image: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(short, long, default_value = ".")]
        out: PathBuf,
    },
    /// Run the pipeline and print a terminal swatch only, no file output
    Preview {
        image: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
    /// Inspect or create the XDG config file
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Print raw k-means clusters (weight/oklch/hex) for tuning min_cluster_weight and hue_tolerance
    Clusters {
        image: PathBuf,
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Print the resolved config path (may not exist yet)
    Path,
    /// Write a fully-commented default config to the XDG config path
    Init {
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
    },
}

struct Pipeline {
    primitives: palette_gen::Primitives,
    resolved: std::collections::BTreeMap<String, palette_gen::Swatch>,
    resolved_component: std::collections::BTreeMap<String, palette_gen::Swatch>,
}

fn run_pipeline(image: &std::path::Path, cfg: &Config) -> Result<Pipeline> {
    let points = extract::load_and_sample(image, cfg.max_dim)?;
    let clusters = extract::kmeans_oklab(&points, cfg.k, cfg.max_iters);
    let stats = extract::image_stats(&clusters);
    let params = cfg.gen_params();
    let primitives = palette_gen::build_primitives(&clusters, &stats, &params);

    let mapping_toml = match &cfg.semantic {
        Some(custom) => std::fs::read_to_string(custom)?,
        None => semantic::DEFAULT_SEMANTIC_TOML.to_string(),
    };
    let mapping = semantic::parse_mapping(&mapping_toml)?;
    let resolved = semantic::resolve(&mapping, &primitives, &stats)?;

    let component_toml = match &cfg.component {
        Some(custom) => std::fs::read_to_string(custom)?,
        None => component::DEFAULT_COMPONENT_TOML.to_string(),
    };
    let component_mapping = component::parse_mapping(&component_toml)?;
    let resolved_component = component::resolve(&component_mapping, &resolved)?;

    Ok(Pipeline {
        primitives,
        resolved,
        resolved_component,
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Generate { image, config, out } => {
            let cfg = Config::load(config.as_deref())?;
            let pipeline = run_pipeline(&image, &cfg)?;

            std::fs::create_dir_all(&out)?;
            std::fs::write(
                out.join("primitives.json"),
                serde_json::to_string_pretty(&pipeline.primitives)?,
            )?;
            std::fs::write(
                out.join("semantic.json"),
                serde_json::to_string_pretty(&pipeline.resolved)?,
            )?;
            std::fs::write(
                out.join("component.json"),
                serde_json::to_string_pretty(&pipeline.resolved_component)?,
            )?;
            println!(
                "wrote {}/primitives.json, {}/semantic.json, and {}/component.json",
                out.display(),
                out.display(),
                out.display()
            );
            preview::print_terminal_mock(&pipeline.resolved);
            preview::print_ansi16(&pipeline.resolved);
        }
        Command::Preview { image, config } => {
            let cfg = Config::load(config.as_deref())?;
            let pipeline = run_pipeline(&image, &cfg)?;
            preview::print_terminal_mock(&pipeline.resolved);
            preview::print_ansi16(&pipeline.resolved);
        }
        Command::Config { action } => match action {
            ConfigAction::Path => {
                println!("{}", xdg::default_config_path().display());
            }
            ConfigAction::Init { force } => {
                let path = xdg::default_config_path();
                if path.exists() && !force {
                    anyhow::bail!(
                        "{} already exists; pass --force to overwrite",
                        path.display()
                    );
                }
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&path, Config::annotated_default_toml())?;
                println!("wrote {}", path.display());
            }
        },
        Command::Clusters { image, config } => {
            let cfg = Config::load(config.as_deref())?;
            let points = extract::load_and_sample(&image, cfg.max_dim)?;
            let clusters = extract::kmeans_oklab(&points, cfg.k, cfg.max_iters);
            let stats = extract::image_stats(&clusters);
            println!(
                "mean_l={:.3} mean_c={:.3} is_dark={}",
                stats.mean_l, stats.mean_c, stats.is_dark
            );
            print!("hue anchors:");
            for slot in palette_gen::HueSlot::ALL {
                print!(" {}={:.1}", slot.name(), slot.anchor_hue());
            }
            println!();
            let mut sorted = clusters.clone();
            sorted.sort_by(|a, b| b.weight.partial_cmp(&a.weight).unwrap());
            println!("{:>7} {:>7} {:>7} {:>7}  hex", "weight", "L", "C", "h");
            for c in &sorted {
                let lch = color::oklab_to_oklch(c.oklab);
                let rgb = color::oklab_to_srgb_u8(c.oklab);
                println!(
                    "{:>6.1}% {:>7.3} {:>7.3} {:>7.1}  {}",
                    c.weight * 100.0,
                    lch[0],
                    lch[1],
                    lch[2],
                    color::to_hex(rgb)
                );
            }
        }
    }

    Ok(())
}
