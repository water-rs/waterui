#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]

//! Web/WASM backend for the `WaterUI` framework.
//!
//! This crate hosts the browser renderer prototype. It provides a high-level [`WebApp`]
//! entry point that is responsible for bootstrapping the DOM, injecting the default
//! `WaterUI` styles, and rendering [`waterui_core::View`] trees into HTML nodes.
//!
//! The current implementation focuses on the plumbing required to run inside
//! `wasm32-unknown-unknown` targets. Rendering logic is intentionally minimal and
//! routes every view through a dispatcher that currently terminates with
//! `todo!()` placeholders.

mod app;
mod dom;
mod error;
mod renderer;

pub use app::{WebApp, WebAppBuilder};
pub use error::WebError;
pub use renderer::{WebRenderer, WebRendererState};
