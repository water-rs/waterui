//! Text components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive text rendering and formatting capabilities,
//! including fonts, attributed text, and internationalization support.
//!
//! Note: The `Link` component has been moved to the main `waterui` crate
//! where it can use `robius-open` for URL handling.

#![no_std]

extern crate alloc;

/// Font utilities and definitions.
pub mod font;
/// Syntax highlighting support.
pub mod highlight;
/// Localization and formatting utilities.
pub mod locale;
/// Styled text support for rich text formatting.
pub mod styled;

/// Core text component.
pub mod text;
pub use text::{IntoText, Text, TextConfig, text};
