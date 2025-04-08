//! Badge component for displaying numeric indicators attached to content.
//!
//! The Badge component displays a small numeric indicator attached to another view,
//! commonly used to show counts of items, notifications, or other numeric values
//! that require attention.
//!
//! # Example
//!
//! ```
//! use waterui::components::Badge;
//! use waterui_core::Color;
//! use waterui::components::Button;
//!
//! let badge = Badge::new(5, Button::new("Messages"));
//! ```

use crate::ViewExt;
use waterui_core::Color;
use waterui_core::{AnyView, View};
use waterui_reactive::{Compute, Computed, compute::IntoComputed};

/// Configuration for the Badge component
#[derive(Debug)]
pub struct BadgeConfig {
    /// The numeric value to display on the badge
    pub value: Computed<i32>,
    /// The content that the badge will be attached to
    pub content: AnyView,
    /// The color of the badge
    pub color: Computed<Color>,
}

configurable!(Badge, BadgeConfig);

impl Badge {
    /// Creates a new Badge with the specified value and content
    ///
    /// # Arguments
    /// * `value` - The numeric value to display on the badge
    /// * `content` - The content that the badge will be attached to
    pub fn new(value: impl IntoComputed<i32>, content: impl View) -> Self {
        Self(BadgeConfig {
            value: value.into_computed(),
            content: content.anyview(),
            color: Color::default().into_computed(),
        })
    }

    /// Sets the color of the badge
    ///
    /// # Arguments
    /// * `color` - The color to use for the badge
    pub fn color(mut self, color: impl Compute<Output = Color>) -> Self {
        self.0.color = color.into_computed();
        self
    }
}
