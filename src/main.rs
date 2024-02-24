

use camino::Utf8PathBuf;
use clap::Parser;
use igraph::dependencies::configfile::parse_config_file;

use tokio::{
    fs::File,
    io::{self},
};
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();

    let args = Args::parse();

    let data = tokio::fs::read_to_string(&args.config).await?;
    let graph = parse_config_file(&data).await?;

    match args.output {
        Some(path) => {
            graph.write_dot(File::open(path).await?).await?;
        }
        None => {
            graph.write_dot(io::stdout()).await?;
        }
    };

    Ok(())
}
