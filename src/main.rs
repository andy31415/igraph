use camino::Utf8PathBuf;
use clap::Parser;
use color_eyre::{eyre::WrapErr, Result};
use include_graph::dependencies::configfile::build_graph;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

/// A program generating DOT graphs for include dependencies.
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// Input configuration file to generate the dot for
    #[arg(short, long)]
    config: Utf8PathBuf,

    /// Where the dot file output should go. Defaults to stdout if not set.
    #[arg(short, long)]
    output: Option<Utf8PathBuf>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::WARN.into())
                    .from_env_lossy(),
            )
            .finish(),
    )
    .unwrap();
    color_eyre::install()?;

    let args = Args::parse();

    let data = std::fs::read_to_string(&args.config)
        .wrap_err_with(|| format!("Failed to open {:?}", &args.config))?;
    let graph = build_graph(&data)?;

    match args.output {
        Some(path) => {
            graph
                .write_dot(
                    std::fs::File::create(&path)
                        .wrap_err_with(|| format!("Failed to create {:?}", path))?,
                )
                .wrap_err_with(|| format!("Failed to write into {:?}", path))?;
        }
        None => {
            graph
                .write_dot(std::io::stdout())
                .wrap_err("Failed to write to stdout")?;
        }
    };

    Ok(())
}
