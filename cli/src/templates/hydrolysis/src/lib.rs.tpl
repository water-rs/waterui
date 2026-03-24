//! Hydrolysis web entry point for {{ ctx.app_display_name }}.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use waterui::env::Environment;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    let app = {{ ctx.crate_name_ident() }}::app(Environment::new());
    hydrolysis::run(app);
}
