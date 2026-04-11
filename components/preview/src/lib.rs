//! WaterUI Preview Crate
//!
//! Provides the `Preview` view component for rendering and capturing WaterUI views.
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

    let _ = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_target(false)
        .with_writer(std::io::stderr)
        .try_init();
}
