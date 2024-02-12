use std::time::Duration;

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

    let v = crate::igraph::parse_compile_database(compile_database).unwrap();

    /*
    for p in v.iter().take(5) {
        println!("Item: {:#?}", p);

        let includes =
            crate::igraph::extract_includes(&p.file_path, &p.include_directories).unwrap();
        println!("   Includes: {:#?}", includes);
    }
    */
    
    tokio::time::sleep(Duration::from_secs(2)).await;

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
    let (items, set_items) = create_signal(TestData::default());
    let items_action = create_action(|_| get_items());

    create_effect(move |_| {
        if let Some(v) = items_action.input().get() {
            println!(".... INPUT seems to have value {}", v);
        } else {
            println!(".... INPUT DOES NOT HAVE A VALUE");
        }
    });

    create_effect(move |_| {
        if let Some(v) = items_action.value().get() {
            println!(".... RESULT seems to have value {:#?}", v);
            if let Ok(td) = v {
                set_items.update(|i| {
                    *i = td;
                });
            }
        } else {
            println!(".... RESULT DOES NOT HAVE A VALUE");
        }
    });

    let on_get_items = move |_| items_action.dispatch("DISPATCH INPUT");
    
    let my_items = create_resource(|| (), |value| async move {
        get_items().await
    });

    view! {
        <h1>"Welcome to Leptos!"</h1>
        <button on:click=on_get_items>"Load compile database"</button>
        <button on:click={move |_|{my_items.refetch()}}>"Load2"</button>

        <p>
            <h3>Items</h3>
            <ul class="file-paths">
               <For
                   each = move|| {items.get().items}
                   key=|item| item.clone()
                   children=move|item| { view!{<li>{item}</li>}}
               />
            </ul>
            <h3>"Items style 2"</h3>
            <Suspense
                fallback=move || view!{<p>"Suspense loading..."</p>}
            >
               {move || match my_items.get() {
                  None => view!{<div>"Loading..."</div>}.into_view(),
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
