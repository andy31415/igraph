use crate::error_template::{AppError, ErrorTemplate};
use leptos::*;
use leptos_meta::*;
use leptos_router::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct TestData {
    pub items: Vec<String>,
}

#[server(GetItems, "/api")]
pub async fn get_items() -> Result<TestData, ServerFnError> {
    let compile_database =
        "/home/andrei/devel/connectedhomeip/out/linux-x64-all-clusters-clang/compile_commands.json";

    let v = crate::igraph::compiledb::parse_compile_database(compile_database)
        .await
        .unwrap();

    Ok(TestData {
        items: v
            .iter()
            .map(|e| e.file_path.to_string_lossy().into())
            .collect(),
    })
}

#[component]
pub fn App() -> impl IntoView {
    // Provides context that manages stylesheets, titles, meta tags, etc.
    provide_meta_context();

    view! {


        // injects a stylesheet into the document <head>
        // id=leptos means cargo-leptos will hot-reload this stylesheet
        <Stylesheet id="leptos" href="/pkg/igraph.css"/>

        // sets the document title
        <Title text="Welcome to Leptos"/>

        // content for this welcome page
        <Router fallback=|| {
            let mut outside_errors = Errors::default();
            outside_errors.insert_with_default_key(AppError::NotFound);
            view! {
                <ErrorTemplate outside_errors/>
            }
            .into_view()
        }>
            <main>
                <Routes>
                    <Route path="" view=HomePage/>
                </Routes>
            </main>
        </Router>
    }
}

/// Renders the home page of your application.
#[component]
fn HomePage() -> impl IntoView {
    let items = create_resource(|| (), |_| async move { get_items().await });

    view! {
        <h1>"Welcome to Leptos!"</h1>
        <button on:click={move |_|{items.refetch()}}>"(Re)load items"</button>

        <p>
            <h3>Items</h3>
            <Suspense fallback=move || view!{<p>"Suspense loading..."</p>} >
               {move || match items.get() {
                  Some(Ok(data)) => view!{
                   <ul class="file-paths">
                   <For
                       each = move|| {data.items.clone()}
                       key=|item| item.clone()
                       children=move|item| { view!{<li>{item}</li>}}
                   />
                  </ul>
                  }.into_view(),
                  _ => view!{<p>"ERROR???..."</p>}.into_view(),
                }
               }
            </Suspense>
        </p>
    }
}
