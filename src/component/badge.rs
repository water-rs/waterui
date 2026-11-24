//! Badge component for displaying indicators attached to content.
//!
//! The Badge component displays a small indicator attached to another view,
//! commonly used to show unread counts, status dots, or short labels that
//! require attention. Zero counts and [`BadgeValue::Hidden`] hide the badge.
//!
//! # Example
//!
//! ```
//! use waterui::prelude::*;
//!
//! let badge = badge::Badge::new(5_u32, button("Messages").action(|| {}));
//! ```

use nami::{Computed, Signal, signal::IntoComputed};
pub use waterui_controls::BadgeValue;
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_graphics::color::Color;

/// Configuration for the Badge component.
#[derive(Debug, Clone)]
pub struct BadgeConfig<S> {
    /// The indicator value to display on the badge.
    pub value: Computed<BadgeValue>,
    /// The content that the badge will be attached to.
    pub content: S,
    /// The color of the badge.
    pub color: Computed<Color>,
}

impl<S: 'static> NativeView for BadgeConfig<S> {}

impl<S: 'static> MapSlots<S> for BadgeConfig<S> {
    type With<T: 'static> = BadgeConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> BadgeConfig<T> {
        BadgeConfig {
            value: self.value,
            content: f(self.content),
            color: self.color,
        }
    }
}

/// A small indicator overlaid on top of another view.
///
/// Badge is typically used to show notification counts, dots, or short text
/// overlaid on icons or buttons. Zero counts and [`BadgeValue::Hidden`] hide
/// the indicator.
///
/// # Layout Behavior
///
/// Badge sizes itself to fit the content it wraps, plus the badge indicator.
/// It never stretches to fill extra space.
#[derive(Debug, Clone)]
#[must_use]
pub struct Badge<C>(BadgeConfig<C>);

impl<C: View> Badge<C> {
    /// Creates a new Badge with the specified value and content.
    ///
    /// The badge color defaults to the theme's [`Error`](crate::theme::color::Error)
    /// slot; backends render the platform error color unless `.color(..)` overrides it.
    ///
    /// # Arguments
    /// * `value` - The badge indicator (count, dot, text, hidden, or signal)
    /// * `content` - The content that the badge will be attached to
    pub fn new(value: impl IntoComputed<BadgeValue>, content: C) -> Self {
        Self(BadgeConfig {
            value: value.into_computed(),
            content,
            color: Color::new(crate::theme::color::Error).into_computed(),
        })
    }

    /// Sets the color of the badge
    ///
    /// # Arguments
    /// * `color` - The color to use for the badge
    pub fn color(mut self, color: impl Signal<Output = Color>) -> Self {
        self.0.color = color.into_computed();
        self
    }

    fn into_config<S: ChildSlot>(self) -> BadgeConfig<S> {
        BadgeConfig {
            value: self.0.value,
            content: S::from_view(self.0.content),
            color: self.0.color,
        }
    }
}

impl<C: View> Badge<C> {
    /// Returns the configuration for this badge.
    #[must_use]
    pub fn config(self) -> BadgeConfig<C> {
        self.0
    }
}

impl<S: View> From<BadgeConfig<S>> for Badge<S> {
    fn from(value: BadgeConfig<S>) -> Self {
        Self(value)
    }
}

impl<C: View> View for Badge<C> {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>())
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(), env, cx)
    }
}
