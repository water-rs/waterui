//! Text components and utilities for the `WaterUI` framework.
//!
//! This crate provides comprehensive text rendering and formatting capabilities,
//! including fonts, attributed text, links, and internationalization support.

/// Font utilities and definitions.
pub mod font;
use font::Font;
/// Attributed text support for rich text formatting.
pub mod attributed;
/// Link components for interactive text.
pub mod link;
/// Localization and formatting utilities.
pub mod locale;
/// Macros for convenient text creation.
#[macro_use]
pub mod macros;
extern crate alloc;

use alloc::string::ToString;
use core::fmt::Display;
use locale::Formatter;
use waterui_core::configurable;

use nami::SignalExt;
use nami::zip::FlattenMap;
use nami::{Computed, Signal, signal::IntoComputed};

use waterui_core::Str;

configurable!(Text, TextConfig);

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Configuration for text components.
///
/// This struct contains all the properties needed to render text,
/// including the content string and font styling information.
pub struct TextConfig {
    /// The text content to be displayed.
    pub content: Computed<Str>,
    /// The font styling to apply to the text.
    pub font: Computed<Font>,
}

impl Clone for Text {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl core::cmp::PartialEq for Text {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl core::cmp::PartialOrd for Text {
    fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
        None
    }
}

impl Default for Text {
    fn default() -> Self {
        text("")
    }
}

impl Text {
    /// Creates a new text component with the given content.
    ///
    /// The text will use default font styling which can be customized
    /// using the builder methods.
    pub fn new(content: impl IntoComputed<Str>) -> Self {
        Self(TextConfig {
            content: content.into_computed(),
            font: Computed::default(),
        })
    }

    /// Creates a text component from any type implementing `Display`.
    ///
    /// This is a convenience method for creating text from values like
    /// numbers, booleans, or other displayable types.
    pub fn display<T: Display>(source: impl IntoComputed<T>) -> Self {
        Self::new(source.into_signal().map(|value| value.to_string()))
    }

    /// Creates a text component using a custom formatter.
    ///
    /// This allows for specialized formatting of values, such as
    /// locale-specific number or date formatting.
    pub fn format<T>(value: impl IntoComputed<T>, formatter: impl Formatter<T> + 'static) -> Self {
        Self::new(
            value
                .into_signal()
                .map(move |value| formatter.format(&value)),
        )
    }

    /// Returns the computed content of this text component.
    ///
    /// This provides access to the reactive text content that will
    /// automatically update when the underlying data changes.
    #[must_use]
    pub fn content(&self) -> Computed<Str> {
        self.0.content.clone()
    }

    /// Sets the font for this text component.
    ///
    /// This allows customizing the typography, including size, weight,
    /// style, and other font properties.
    #[must_use]
    pub fn font(mut self, font: impl Signal<Output = Font>) -> Self {
        self.0.font = font.computed();
        self
    }

    /// Sets the font size for this text component.
    ///
    /// This is a convenience method for setting just the size while
    /// preserving other font properties.
    #[must_use]
    pub fn size(mut self, size: impl IntoComputed<f64>) -> Self {
        self.0.font = self
            .0
            .font
            .zip(size.into_signal())
            .flatten_map(|mut old, size| {
                old.size = size;
                old
            })
            .computed();
        self
    }
}

/// Creates a new text component with the given content.
///
/// This is a convenience function equivalent to `Text::new(text)`.
///
/// # Tip
/// If you need formatted text, please use the `text!` macro instead.
#[must_use]
pub fn text(text: impl IntoComputed<Str>) -> Text {
    Text::new(text)
}

impl<T> From<T> for Text
where
    T: IntoComputed<Str>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

pub use nami as __nami;
