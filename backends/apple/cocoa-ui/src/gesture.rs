//! Shared vocabulary for the gesture recognizers in [`crate::appkit`] and
//! [`crate::uikit`].
//!
//! The twins' recognizer constructors differ — each platform has its own
//! recognizer classes — but they describe their configuration in the same
//! terms: the pointer buttons a gesture responds to, and the state a
//! recognizer reports when it fires.
//!
//! # Safety
//!
//! This module is pure data; it performs no Objective-C calls.

/// The pointer buttons a gesture responds to, as a bit set.
///
/// The bit order follows the DOM `buttons` convention — the same order
/// `UIEvent.ButtonMask` and `AppKit`'s `buttonMask` use: bit 0 primary, bit 1
/// secondary, bit 2 middle, bit 3 back, bit 4 forward. A touch or a pen
/// contact reports [`ButtonMask::PRIMARY`], so a primary-only mask still
/// accepts a finger.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ButtonMask(u8);

impl ButtonMask {
    /// The main button: a left click, a touch, a pen contact.
    pub const PRIMARY: Self = Self(1 << 0);
    /// The secondary button, usually a right click.
    pub const SECONDARY: Self = Self(1 << 1);
    /// The middle button, usually a wheel click.
    pub const MIDDLE: Self = Self(1 << 2);
    /// The "back" side button.
    pub const BACK: Self = Self(1 << 3);
    /// The "forward" side button.
    pub const FORWARD: Self = Self(1 << 4);

    /// The mask with these exact bits, in DOM `buttons` order.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// The mask's bits, in DOM `buttons` order.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether any button of `other` is in this set.
    #[must_use]
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }
}

impl Default for ButtonMask {
    fn default() -> Self {
        Self::PRIMARY
    }
}

impl core::ops::BitOr for ButtonMask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

/// Where a gesture recognizer is in its recognition cycle, normalized across
/// `NSGestureRecognizerState` and `UIGestureRecognizerState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GestureState {
    /// No recognition has happened yet.
    Possible,
    /// The gesture began — a continuous gesture's first event.
    Began,
    /// The gesture is still updating.
    Changed,
    /// The gesture recognized and completed.
    Ended,
    /// The gesture was cancelled mid-stream.
    Cancelled,
    /// The gesture failed to recognize.
    Failed,
}
