use tracing_subscriber::{EnvFilter, FmtSubscriber};


#[tokio::main]
async fn main() {
    tracing::subscriber::set_global_default(
        FmtSubscriber::builder()
            .with_env_filter(EnvFilter::from_default_env())
            .finish(),
    )
    .unwrap();

    
    println!("TODO: this needs to be implemented.");
}
