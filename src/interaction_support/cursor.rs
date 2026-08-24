//! Cursor style metadata for views.
//!
//! This module provides cursor style customization for views on platforms
//! that support pointer cursors (macOS, iPadOS with trackpad, Android API 24+).
//!
//! # Example
//!
//! ```rust
//! use waterui::prelude::*;
//! use waterui::cursor::CursorStyle;
//!
//! // Static cursor
//! text!("Click me").cursor(CursorStyle::PointingHand)
//!
//! // Reactive cursor based on state
//! let is_dragging = Binding::bool(false);
//! let is_dragging_for_cursor = is_dragging.clone();
//! view.cursor(Computed::new(move || if is_dragging_for_cursor.get() {
//!     CursorStyle::ClosedHand
//! } else {
//!     CursorStyle::OpenHand
//! }))
//! ```

use nami::{Computed, impl_constant, signal::IntoComputed};
use waterui_core::metadata::MetadataKey;

/// Cursor styles that can be displayed when hovering over a view.
///
/// The cursor style is automatically reset when the cursor exits the view's bounds.
/// Not all styles may be available on all platforms - unavailable styles typically
/// fall back to the default arrow cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum CursorStyle {
    /// Default arrow cursor (system default behavior).
    #[default]
    Arrow,
    /// Pointing hand cursor (for clickable/link elements).
    PointingHand,
    /// Text selection cursor (I-beam).
    IBeam,
    /// Crosshair cursor (for precise selection).
    Crosshair,
    /// Open hand cursor (for draggable content).
    OpenHand,
    /// Closed hand cursor (while dragging).
    ClosedHand,
    /// Not-allowed cursor (for disabled actions).
    NotAllowed,
    /// Resize left cursor.
    ResizeLeft,
    /// Resize right cursor.
    ResizeRight,
    /// Resize up cursor.
    ResizeUp,
    /// Resize down cursor.
    ResizeDown,
    /// Resize left-right cursor (horizontal resize).
    ResizeLeftRight,
    /// Resize up-down cursor (vertical resize).
    ResizeUpDown,
    /// Move cursor (for movable content).
    Move,
    /// Wait/loading cursor.
    Wait,
    /// Copy cursor (for copy operations).
    Copy,
}

impl_constant!(CursorStyle);

/// Metadata to set the cursor style when hovering over a view.
///
/// The cursor style is scoped to the view's bounds - when the cursor exits
/// the view, the cursor automatically reverts to the parent view's cursor
/// or the system default.
///
/// # Platform Support
///
/// - **macOS**: Full support via `NSCursor`
/// - **iOS**: Not applicable (no visible cursor)
/// - **iPadOS**: Supported with external trackpad via `UIPointerStyle`
/// - **Android**: Supported on API 24+ via `View.pointerIcon`
#[derive(Debug)]
pub struct Cursor {
    /// The cursor style to display, can be reactive.
    pub style: Computed<CursorStyle>,
}

impl MetadataKey for Cursor {}

impl Cursor {
    /// Creates a new cursor metadata with the given style.
    #[must_use]
    pub fn new(style: impl IntoComputed<CursorStyle>) -> Self {
        Self {
            style: style.into_computed(),
        }
    }
}
