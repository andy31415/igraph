use clap::Parser;

use leptos::{component, view, IntoView};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use axum::Router;
use igraph::{self, extract_includes, parse_compile_database};
use leptos::get_configuration;
use leptos_axum::{generate_route_list, LeptosRoutes};

/// Generates graphs of C++ includes
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Name of the person to greet
    #[arg(short, long)]
    compile_database: String,
}

#[component]
fn App() -> impl IntoView {
    view! {
        <main>
            <HelloComponent name="Andrei".to_string() />
        </main>
    }
}

#[component]
fn HelloComponent(name: String) -> impl IntoView {
    view! {
        <p>Hello {name}</p>
    }
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

    let conf = get_configuration(None).await.unwrap();
    let leptos_options = conf.leptos_options;
    let addr = leptos_options.site_addr;
    let routes = generate_route_list(App);

    // build our application with a route
    let app = Router::new()
        .leptos_routes(&leptos_options, routes, App)
        .with_state(leptos_options);

    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    println!("Listen on http://{}", &addr);
    axum::serve(listener, app).await.unwrap();

    Ok(())
}
