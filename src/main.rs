use clap::Parser;

use tracing_subscriber::{
    filter::LevelFilter,
    layer::SubscriberExt,
    util::SubscriberInitExt, Layer};

/// Generates graphs of C++ includes
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    combpile_database: String,
    
    #[arg(short, long)]
    log_level: Option<LevelFilter>,
}

fn main() {
    let args = Args::parse();

    let stdout_log = tracing_subscriber::fmt::layer().compact();
    tracing_subscriber::registry()
        .with(stdout_log.with_filter(args.log_level.unwrap_or(LevelFilter::TRACE)))
        .init();
}