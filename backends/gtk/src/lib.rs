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
#[cfg(target_os = "linux")]
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
mod theme;
#[cfg(target_os = "linux")]
pub mod util;
// The only web engine this backend bridges is the platform's own, `WebKitGTK`.
// Every other engine is a crate the application links and installs, and it
// reaches the screen as an ordinary `GpuSurface` this backend draws like any
// other — see `components::graphics::gpu_surface`.
#[cfg(all(target_os = "linux", feature = "webview-system"))]
#[path = "webview_system.rs"]
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
