//! WaterUI Preview Crate
//!
//! Provides the `Preview` view component for rendering and capturing WaterUI views.
//!
//! This crate is used by the preview daemon app at `~/.water/preview_app/`
//! to handle preview requests from the CLI via TCP.
//!
//! ## Architecture
//!
//! ```text
//! CLI (water preview) → TCP → Preview Daemon App (uses this crate) → PNG capture
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
pub mod protocol;
pub mod renderer;
mod tcp;
mod view;

pub use library::{LoadError, PreviewLibrary};
pub use protocol::{DylibSource, PreviewError, PreviewOutput, PreviewRequest, PreviewResponse, Size};
pub use renderer::{CustomViewRenderer, RenderResult, RenderResultExt, RenderSize, ViewRenderer};
pub use tcp::{RequestHandler, TcpServer};
pub use view::Preview;
