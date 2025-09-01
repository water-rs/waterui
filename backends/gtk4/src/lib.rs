//! GTK4 backend for WaterUI framework.
//!
//! This crate provides a GTK4 implementation for WaterUI components, enabling
//! native desktop applications on Linux, Windows, and macOS using the GTK4 toolkit.
//!
//! # Usage
//!
//! ```rust
//! use waterui_gtk4::{Gtk4App, Gtk4Window};
//! use waterui_core::View;
//!
//! #[tokio::main]
//! async fn main() {
//!     let app = Gtk4App::new("com.example.waterui-app");
//!     
//!     app.run(|| {
//!         Gtk4Window::new().content(|| {
//!             text("Hello, GTK4!")
//!         })
//!     }).await;
//! }
//! ```

pub mod app;
pub mod events;
pub mod layout;
pub mod renderer;
pub mod widgets;

pub use app::Gtk4App;
// pub use renderer::Gtk4Renderer;

use gtk4 as gtk;

/// Initialize the GTK4 backend.
///
/// This should be called once at the beginning of your application
/// before creating any GTK4 widgets.
pub fn init() -> Result<(), Box<dyn std::error::Error>> {
    gtk::init()?;
    Ok(())
}
