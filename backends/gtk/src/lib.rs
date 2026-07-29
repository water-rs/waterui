//! GTK4 backend for `WaterUI`.
//!
//! This crate provides a GTK4-based rendering backend for `WaterUI`, mapping
//! `WaterUI` views to native GTK4 widgets.
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

// NOTE: The GTK backend is currently supported only on Linux.
// On other platforms this crate compiles as an empty crate so workspace builds succeed.

#[cfg(target_os = "linux")]
pub mod app;
#[cfg(all(
    target_os = "linux",
    any(feature = "webview-cef", feature = "chromium")
))]
mod browser_cef;
#[cfg(all(
    target_os = "linux",
    any(feature = "webview-wpe", feature = "webview-cef", feature = "chromium")
))]
mod browser_input;
#[cfg(target_os = "linux")]
pub mod component;
#[cfg(target_os = "linux")]
pub mod components;
#[cfg(target_os = "linux")]
pub mod layout;
#[cfg(target_os = "linux")]
pub mod renderer;
#[cfg(target_os = "linux")]
pub mod util;
#[cfg(all(
    target_os = "linux",
    feature = "webview-system",
    any(feature = "webview-wpe", feature = "webview-cef")
))]
compile_error!("GTK WebView selects exactly one of `system`, `wpe`, or `cef`");
#[cfg(all(target_os = "linux", feature = "webview-wpe", feature = "webview-cef"))]
compile_error!("GTK WebView selects exactly one of `system`, `wpe`, or `cef`");

#[cfg(all(target_os = "linux", feature = "webview-system"))]
#[path = "webview_system.rs"]
pub mod webview;
#[cfg(all(
    target_os = "linux",
    feature = "webview-wpe",
    not(feature = "webview-system"),
    not(feature = "webview-cef")
))]
#[path = "webview_wpe.rs"]
pub mod webview;
#[cfg(all(
    target_os = "linux",
    feature = "webview-cef",
    not(feature = "webview-system"),
    not(feature = "webview-wpe")
))]
#[path = "webview_cef.rs"]
pub mod webview;
#[cfg(target_os = "linux")]
pub mod window;

#[cfg(target_os = "linux")]
pub use app::GtkApp;
#[cfg(target_os = "linux")]
pub use app::init_main_thread_executors;
#[cfg(target_os = "linux")]
pub use renderer::GtkRenderer;

// Re-export types needed by generated GTK entry points
#[cfg(target_os = "linux")]
pub use waterui_core::Environment;
