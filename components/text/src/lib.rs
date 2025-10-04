//! Text components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive text rendering and formatting capabilities,
//! including fonts, attributed text, links, and internationalization support.

/// Font utilities and definitions.
pub mod font;
/// Styled text support for rich text formatting.
pub mod styled;
pub mod highlight;
/// Link components for interactive text.
pub mod link;
/// Localization and formatting utilities.
pub mod locale;
/// Macros for convenient text creation.
#[macro_use]
pub mod macros;
extern crate alloc;

mod text;
pub use text::{Text,TextConfig,text};

pub use nami as __nami;
