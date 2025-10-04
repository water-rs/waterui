
use alloc::string::ToString;
use nami::impl_constant;
use nami::signal::IntoSignal;
use core::fmt::Display;
use crate::font::FontWeight;
use crate::{font::Font, styled::StyledStr};
use crate::locale::Formatter;
use nami::{Computed, Signal, SignalExt, signal::IntoComputed};
use waterui_core::{configurable};


configurable!(Text, TextConfig);

#[derive(Debug, Clone)]
#[non_exhaustive]
/// Configuration for text components.
///
/// This struct contains all the properties needed to render text,
/// including the content string and font styling information.
pub struct TextConfig {
    /// The rich text content to be displayed.
    pub content: Computed<StyledStr>,
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
    /// Creates a new text component.
    pub fn new(content: impl IntoComputed<StyledStr>) -> Self {
        Self(TextConfig {
            content: content.into_signal().map(StyledStr::from).computed(),
        })
    }


    /// Creates a text component from any type implementing `Display`.
    ///
    /// This is a convenience method for creating text from values like
    /// numbers, booleans, or other displayable types.
    pub fn display<T: Display>(source: impl Signal<Output = T>) -> Self {
        Self::new(source.map(|value| value.to_string()))
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
    pub fn content(&self) -> Computed<StyledStr> {
        self.0.content.clone()
    }

    /// Sets the font for this text component.
    ///
    /// This allows customizing the typography, including size, weight,
    /// style, and other font properties.
    #[must_use]
    pub fn font(mut self, font: impl Signal<Output = Font>) -> Self {
        self.0.content = self.0.content.zip(font).map(|(content, font)| {
            content.font(font)
        }).computed();
        self
    }

    /// Sets the font size.
    pub fn size(mut self, size: impl IntoSignal<f64>) -> Self {
        // A litle sad we have to do this conversion here
        let size = size.into_signal().map(|s| s as f32);
        self.0.content = self.0.content.zip(size).map(|(content, size)| {
            content.size(size)
        }).computed();
        self
    }

    /// Sets the font weight.
    pub fn weight(mut self, weight: impl Signal<Output = FontWeight>) -> Self {
        self.0.content = self.0.content.zip(weight).map(|(content, weight)| {
            content.weight(weight)
        }).computed();
        self
    }

    /// Sets the font to bold.
    #[must_use] 
    pub fn bold(self) -> Self{
        self.weight(FontWeight::Bold)
    }

    /// Sets the italic style.
    pub fn italic(mut self, is_italic: impl Signal<Output = bool>) -> Self {
        self.0.content = self.0.content.zip(is_italic).map(|(content, is_italic)| {
            content.italic(is_italic)
        }).computed();
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
pub fn text(text: impl IntoComputed<StyledStr>) -> Text {
    Text::new(text)
}

impl<T> From<T> for Text
where
    T: IntoComputed<StyledStr>,
{
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

impl_constant!(Text,TextConfig);