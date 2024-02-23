pub mod app;
pub mod dependency_graph;
pub mod error_template;

cfg_if::cfg_if! {
if #[cfg(feature = "ssr")] {
pub mod fileserv;
pub mod igraph;
}
}

#[cfg(feature = "ssr")]
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn hydrate() {
    use crate::app::*;
    console_error_panic_hook::set_once();
    leptos::mount_to_body(App);
}
