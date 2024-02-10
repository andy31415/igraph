use clap::Parser;

use tracing_subscriber::{EnvFilter, FmtSubscriber};

use igraph::{self, extract_includes, parse_compile_database};

/// Generates graphs of C++ includes
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    compile_database: String,
}

fn main() -> Result<(), igraph::Error> {
    let args = Args::parse();

    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();

    // Access data using struct fields
    for p in parse_compile_database(&args.compile_database)?
            .iter()
            .take(5) {
        println!( "Item: {:#?}", p);
        
        extract_includes(&p.file_path, &p.include_directories);
    }


    Ok(())
}
