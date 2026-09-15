//! Badge component for displaying numeric indicators attached to content.
//!
//! The Badge component displays a small numeric indicator attached to another view,
//! commonly used to show counts of items, notifications, or other numeric values
//! that require attention.
//!
//! # Example
//!
//! ```
//! use waterui::prelude::*;
//!
//! let badge = badge::Badge::new(5, button("Messages"));
//! ```

use crate::theme::color::AccentForeground;
use crate::widget::condition::when;
use crate::{SignalExt, ViewExt};
use nami::{Computed, Signal, signal::IntoComputed};
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{Environment, View};
use waterui_graphics::color::signal_color;
use waterui_graphics::color::{AccentColor, Color};
use waterui_layout::overlay;
use waterui_layout::padding::EdgeInsets;
use waterui_layout::stack::Alignment;
use waterui_macros::text;
use waterui_shape::{Capsule, Circle};

/// Side length of the dot rendered for a non-positive count, in points.
const DOT_SIZE: f32 = 6.0;

/// How far the indicator hangs past the content's top-trailing corner, in
/// points. Platform badges sit half outside the badged view rather than fully
/// inside its bounds.
const INDICATOR_OFFSET: f32 = 6.0;

/// Configuration for the Badge component
#[derive(Debug)]
pub struct BadgeConfig {
    /// The numeric value to display on the badge
    pub value: Computed<i32>,
    /// The content that the badge will be attached to
    pub content: AnyViewBuilder,
    /// The color of the badge
    pub color: Computed<Color>,
}

/// A small indicator that displays a count on top of another view.
///
/// Badge is typically used to show notification counts or item quantities
/// overlaid on icons or buttons. A positive count renders a capsule carrying
/// the number; zero or negative counts render a bare dot, the convention for
/// "there is something here but nothing countable".
///
/// # Layout Behavior
///
/// Badge sizes itself to fit the content it wraps; the indicator overlays the
/// content's top-trailing corner, nudged [`INDICATOR_OFFSET`] points outward,
/// and never stretches to fill extra space.
///
/// This is a Rust-side composer — a stack, an overlay, a clipped capsule and
/// theme tokens — and ships no FFI type of its own, so it renders on every
/// backend without a native leaf.
#[derive(Debug)]
pub struct Badge(BadgeConfig);

impl Badge {
    /// Creates a new Badge with the specified value and content
    ///
    /// # Arguments
    /// * `value` - The numeric value to display on the badge
    /// * `content` - The content that the badge will be attached to
    pub fn new(value: impl IntoComputed<i32>, content: impl View + Clone) -> Self {
        Self(BadgeConfig {
            value: value.into_computed(),
            content: AnyViewBuilder::new(move || content.clone().anyview()),
            color: Color::new(AccentColor).into_computed(),
        })
    }

    /// Sets the color of the badge
    ///
    /// # Arguments
    /// * `color` - The color to use for the badge
    #[must_use]
    pub fn color(mut self, color: impl Signal<Output = Color>) -> Self {
        self.0.color = color.into_computed();
        self
    }
}

impl View for Badge {
    fn body(self, _env: &Environment) -> impl View {
        let BadgeConfig {
            value,
            content,
            color,
        } = self.0;

        let pill_color = color.clone();
        let has_count = value.map(|count| count > 0);
        // `when` re-invokes its builders whenever the condition flips, so each
        // call site clones the signals it binds rather than consuming them.
        let indicator = when(has_count, move || {
            let value = value.clone();
            text!("{value}")
                .caption()
                .bold()
                .foreground(AccentForeground)
                .padding_with(EdgeInsets::symmetric(1.0, 4.0))
                .background(signal_color(pill_color.clone()))
                .clip(Capsule)
        })
        .otherwise(move || {
            signal_color(color.clone())
                .size(DOT_SIZE, DOT_SIZE)
                .clip(Circle)
        });

        overlay(
            content.build(),
            indicator.offset(INDICATOR_OFFSET, -INDICATOR_OFFSET),
        )
        .alignment(Alignment::TopTrailing)
    }
}
