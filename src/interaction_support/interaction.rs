//! Interaction modifiers for controlling view interactivity.
//!
//! This module provides modifiers for controlling how views respond to user interactions
//! such as touch events and hit testing.

use nami::{Computed, signal::IntoComputed};
use waterui_core::metadata::MetadataKey;

/// Controls whether a view responds to hit testing (touch/click events).
///
/// When `enabled` is false, touch events pass through the view to views behind it.
/// This modifier does NOT affect the visual appearance of the view.
///
/// # Example
///
/// ```rust,ignore
/// use waterui::prelude::*;
///
/// // Make a view transparent to touch events
/// text("Click through me")
///     .hittable(false);
///
/// // Reactive hit testing control
/// let can_interact = binding(true);
/// button("Click me").action(|| {})
///     .hittable(can_interact);
/// ```
#[derive(Debug)]
pub struct Hittable {
    /// Whether hit testing is enabled.
    /// `true` means the view responds to touch events (default behavior).
    /// `false` means touch events pass through the view.
    pub enabled: Computed<bool>,
}

impl MetadataKey for Hittable {}

impl Hittable {
    /// Creates a new Hittable modifier.
    ///
    /// # Arguments
    ///
    /// * `enabled` - Whether hit testing is enabled (can be reactive).
    #[must_use]
    pub fn new(enabled: impl IntoComputed<bool>) -> Self {
        Self {
            enabled: enabled.into_computed(),
        }
    }
}
