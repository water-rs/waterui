//! Text components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive text rendering and formatting capabilities,
//! including fonts, attributed text, and internationalization support.
//!
//! Note: The `Link` component has been moved to the main `waterui` crate
//! where it can use `robius-open` for URL handling.

#![allow(clippy::future_not_send)]
#![no_std]

/// Font utilities and definitions.
pub mod font;
/// Syntax highlighting support.
pub mod highlight;
/// Localization and formatting utilities.
pub mod locale;
/// Styled text support for rich text formatting.
pub mod styled;
/// Macros for convenient text creation.
#[macro_use]
pub mod macros;
extern crate alloc;

/// Core text component.
pub mod text;
pub use text::{Text, TextConfig, text};

pub use nami as __nami;
