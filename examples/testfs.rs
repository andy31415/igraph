use igraph::dependencies::configfile::parse_config_file;
use tracing::error;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();

    if let Err(e) = parse_config_file(include_str!("../sample_api.txt")).await {
        error!("PARSE ERROR: {:#?}", e);
    }
}
