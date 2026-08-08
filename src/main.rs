mod color;
mod config;
mod extract;
mod palette_gen;
mod preview;
mod semantic;

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
    /// Run the full pipeline and write primitives.json + semantic.json
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
}

struct Pipeline {
    primitives: palette_gen::Primitives,
    resolved: std::collections::BTreeMap<String, palette_gen::Swatch>,
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

    Ok(Pipeline {
        primitives,
        resolved,
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
            println!(
                "wrote {}/primitives.json and {}/semantic.json",
                out.display(),
                out.display()
            );
            preview::print_ansi16(&pipeline.resolved);
        }
        Command::Preview { image, config } => {
            let cfg = Config::load(config.as_deref())?;
            let pipeline = run_pipeline(&image, &cfg)?;
            preview::print_ansi16(&pipeline.resolved);
        }
    }

    Ok(())
}
