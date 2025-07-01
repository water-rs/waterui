//! This module provides types for defining background and foreground colors in a UI.

use waterui_core::{Color, Computed};
use waterui_reactive::compute::IntoComputed;
use waterui_str::Str;

/// Represents different kinds of backgrounds that can be applied to UI elements.
#[derive(Debug)]
pub enum Background {
    /// A solid color background.
    Color(Computed<Color>),
    /// An image background.
    Image(Computed<Str>),
}

impl Background {
    /// Creates a new background with a solid color.
    ///
    /// # Arguments
    ///
    /// * `color` - A value that can be converted into a computed color.
    ///
    /// # Returns
    ///
    /// A new `Background` instance with the specified color.
    pub fn color(color: impl IntoComputed<Color>) -> Self {
        Self::Color(color.into_computed())
    }
}

/// Represents the color of text or other foreground elements in a UI.
#[derive(Debug)]
pub struct ForegroundColor {
    /// The computed color value.
    pub color: Computed<Color>,
}

impl ForegroundColor {
    /// Creates a new foreground color.
    ///
    /// # Arguments
    ///
    /// * `color` - A value that can be converted into a computed color.
    ///
    /// # Returns
    ///
    /// A new `ForegroundColor` instance with the specified color.
    pub fn new(color: impl IntoComputed<Color>) -> Self {
        Self {
            color: color.into_computed(),
        }
    }
}
