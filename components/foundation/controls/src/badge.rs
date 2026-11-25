//! Shared badge value vocabulary for badge overlays and tab items.

use nami::impl_constant;
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
