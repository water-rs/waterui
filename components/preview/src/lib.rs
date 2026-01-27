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
