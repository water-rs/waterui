//! Hydrolysis web entry point for __APP_DISPLAY_NAME__.

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use waterui::env::Environment;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    let app = __CRATE_NAME_IDENT__::app(Environment::new());
    hydrolysis::run(app);
}
