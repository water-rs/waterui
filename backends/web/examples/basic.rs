use wasm_bindgen::prelude::*;
use waterui::{Environment, component::text};
use waterui_web::{WebApp, init};

#[wasm_bindgen(start)]
#[allow(clippy::main_recursion)]
pub fn main() {
    init();

    let app = WebApp::new("app");
    let env = Environment::new();

    let content = text("Hello, WaterUI Web!");

    if let Err(e) = app.environment(env).render(content) {
        web_sys::console::log_1(&format!("Error rendering app: {:?}", e).into());
    }
}

#[wasm_bindgen]
pub fn greet(name: &str) {
    web_sys::console::log_1(&format!("Hello, {}! This is WaterUI Web backend.", name).into());
}
