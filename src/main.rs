use clap::Parser;

use tracing_subscriber::{
    filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

use igraph::{self, parse_compile_database};

/// Generates graphs of C++ includes
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    compile_database: String,

    #[arg(short, long)]
    log_level: Option<LevelFilter>,
}

fn main() -> Result<(), igraph::Error> {
    let args = Args::parse();

    let stdout_log = tracing_subscriber::fmt::layer().compact();
    tracing_subscriber::registry()
        .with(stdout_log.with_filter(args.log_level.unwrap_or(LevelFilter::WARN)))
        .init();

    // Access data using struct fields
    println!(
        "Item: {:#?}",
        parse_compile_database(&args.compile_database)?
            .iter()
            .take(5)
            .collect::<Vec<_>>()
    );

    Ok(())
}
