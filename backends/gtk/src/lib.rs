//! GTK4 backend for WaterUI.
//!
//! This crate provides a GTK4-based rendering backend for WaterUI, mapping
//! WaterUI views to native GTK4 widgets.
//!
//! # Architecture
//!
//! - Uses native GTK4 widgets (Label, Button, Switch, etc.) where possible
//! - Layout is handled by `waterui-layout`, with GTK only measuring and placing widgets
//! - Reactivity uses `nami::watch()` directly to update widget properties
//!
//! # Example
//!
//! ```ignore
//! use waterui_gtk::GtkBackend;
//!
//! let backend = GtkBackend::new();
//! backend.run(my_view);
//! ```

pub mod app;
pub mod component;
pub mod components;
pub mod layout;
pub mod renderer;
pub mod util;
pub mod webview;
pub mod window;

pub use app::GtkApp;
pub use renderer::GtkRenderer;

// Re-export types needed by generated GTK entry points
pub use waterui_core::Environment;
