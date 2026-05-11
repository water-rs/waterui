//! `WaterUI` Preview Crate
//!
//! Provides the `Preview` view component for rendering and capturing `WaterUI` views.
//!
//! This crate is used by the preview support app scaffolded by the CLI at
//! `~/.water/preview_support/` to handle preview requests via TCP.
//!
//! ## Architecture
//!
//! ```text
//! CLI (water preview) → TCP → Preview Support App (uses this crate) → PNG capture
//! ```
//!
//! ## Usage
//!
//! The preview app simply includes the `Preview` view:
//!
//! ```ignore
//! use waterui::app::App;
//! use waterui::prelude::*;
//! use waterui_preview::Preview;
//!
//! fn main() -> impl View {
//!     Preview::new()
//! }
//!
//! pub fn app(env: Environment) -> App {
//!     App::new(main, env)
//! }
//!
//! waterui_ffi::export!();
//! ```

mod cache;
mod library;
pub mod renderer;
mod view;

pub use library::{LoadError, PreviewLibrary};
pub use renderer::{CustomViewRenderer, RenderResult, RenderResultExt, RenderSize, ViewRenderer};
pub use view::Preview;
pub use waterui_preview_protocol as protocol;
pub use waterui_preview_protocol::{
    DylibId, DylibSource, PreviewError, PreviewOutput, PreviewRequest, PreviewResponse, Size,
};

/// Initialize preview tracing from `RUST_LOG` when the support app wants internal timing logs.
pub fn init_tracing_from_env() {
    use tracing_subscriber::EnvFilter;

    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }

    let log_dir = protocol::registry::preview_cache_root_dir().join("logs");
    std::fs::create_dir_all(&log_dir).unwrap_or_else(|error| {
        panic!(
            "failed to create preview log dir {}: {error}",
            log_dir.display()
        )
    });
    let log_path = log_dir.join(format!("support-app-{}.log", std::process::id()));
    let make_writer = {
        let log_path = log_path.clone();
        move || {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_path)
                .unwrap_or_else(|error| {
                    panic!(
                        "failed to open preview support log {}: {error}",
                        log_path.display()
                    )
                })
        }
    };

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_writer(make_writer)
        .try_init();

    tracing::info!(path = %log_path.display(), "Preview support app tracing initialized");
}
