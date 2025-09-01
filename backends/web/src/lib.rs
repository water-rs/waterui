//! Web backend for WaterUI framework.
//!
//! This crate provides a web/WASM implementation for WaterUI components, enabling
//! web applications that can run in browsers using WebAssembly.
//!
//! # Usage
//!
//! ```rust
//! use waterui_web::{WebApp, WebElement};
//! use waterui_core::View;
//! use wasm_bindgen::prelude::*;
//!
//! #[wasm_bindgen]
//! pub fn run_app() {
//!     let app = WebApp::new("app-container");
//!     
//!     app.render(|| {
//!         text("Hello, Web!")
//!     });
//! }
//! ```

pub mod app;
pub mod element;
pub mod events;
pub mod renderer;
pub mod widgets;

pub use app::WebApp;
pub use element::WebElement;

use wasm_bindgen::prelude::*;

/// Initialize the web backend.
///
/// This should be called once at the beginning of your application
/// before creating any web components.
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
    web_sys::console::log_1(&"WaterUI Web backend initialized".into());
}
