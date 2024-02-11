use clap::Parser;

use tracing_subscriber::{EnvFilter, FmtSubscriber};

use axum::{routing::get, Router};
use igraph::{self, extract_includes, parse_compile_database};

/// Generates graphs of C++ includes
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    compile_database: String,
}

#[tokio::main]
async fn main() -> Result<(), igraph::Error> {
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
        .take(5)
    {
        println!("Item: {:#?}", p);

        let includes = extract_includes(&p.file_path, &p.include_directories).unwrap();
        println!("   Includes: {:#?}", includes);
    }

    // build our application with a route
    let app = Router::new()
        // `GET /` goes to `root`
        .route("/", get(root));

    // run our app with hyper, listening globally on port 3000
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();

    Ok(())
}

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}
