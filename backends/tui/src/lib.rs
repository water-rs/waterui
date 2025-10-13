#![allow(clippy::multiple_crate_versions)]

//! Terminal backend entry point for `WaterUI`.

pub use crate::app::{TuiApp, TuiAppBuilder};
pub use crate::renderer::{RenderFrame, Renderer};
pub use crate::terminal::Terminal;

mod app;
mod error;
mod renderer;
mod terminal;

pub use error::TuiError;
