//! Shared badge value vocabulary and badge overlay component.

use nami::{
    Computed, SignalExt as _, impl_constant,
    signal::{IntoComputed, IntoSignal},
};
use waterui_color::{Color, token::Error as ErrorColor};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_text::{IntoText, Text};

/// The indicator displayed by a badge or tab item.
#[derive(Debug, Clone, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each badge presentation form directly; adding a form must fail backend builds instead of hitting a wildcard"
)]
pub enum BadgeValue {
    /// No badge indicator is displayed.
    #[default]
    Hidden,
    /// A small indicator dot without a count or label.
    Dot,
    /// A numeric count pill (`0` renders hidden; [`From<u32>`] maps `0` to [`Self::Hidden`]).
    Count(u32),
    /// A text label pill.
    Text(Text),
}

impl BadgeValue {
    /// Creates a badge value for `count`, mapping `0` to [`Self::Hidden`].
    #[must_use]
    pub const fn count(count: u32) -> Self {
        if count == 0 {
            Self::Hidden
        } else {
            Self::Count(count)
        }
    }

    /// Creates a text badge from `text`.
    #[must_use]
    pub fn text(text: impl IntoText) -> Self {
        Self::Text(text.into_text())
    }

    /// Returns `true` when this badge renders nothing (`Hidden` or `Count(0)`).
    #[must_use]
    pub const fn is_hidden(&self) -> bool {
        matches!(self, Self::Hidden | Self::Count(0))
    }
}

impl PartialEq for BadgeValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Hidden | Self::Count(0), Self::Hidden | Self::Count(0))
            | (Self::Dot, Self::Dot) => true,
            (Self::Count(a), Self::Count(b)) => a == b,
            (Self::Text(a), Self::Text(b)) => a == b,
            _ => false,
        }
    }
}

impl From<u32> for BadgeValue {
    fn from(value: u32) -> Self {
        Self::count(value)
    }
}

impl From<Text> for BadgeValue {
    fn from(value: Text) -> Self {
        Self::Text(value)
    }
}

impl_constant!(BadgeValue);

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
    /// The badge color defaults to the theme's `Error` slot; backends render
    /// the platform error color unless `.color(..)` overrides it.
    pub fn new(value: impl IntoComputed<BadgeValue>, content: C) -> Self {
        Self(BadgeConfig {
            value: value.into_computed(),
            content,
            color: Color::new(ErrorColor).into_computed(),
        })
    }

    /// Sets the color of the badge.
    pub fn color(mut self, color: impl IntoSignal<Color> + 'static) -> Self {
        self.0.color = color.into_signal().computed();
        self
    }

    fn into_config<S: ChildSlot>(self) -> BadgeConfig<S> {
        BadgeConfig {
            value: self.0.value,
            content: S::from_view(self.0.content),
            color: self.0.color,
        }
    }

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
