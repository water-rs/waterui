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
//! let clickable = text!("Click me").cursor(CursorStyle::PointingHand);
//!
//! // Reactive cursor based on state
//! let is_dragging = binding::<bool>(false);
//! let draggable = text!("Drag me").cursor(is_dragging.map(|dragging| if dragging {
//!     CursorStyle::ClosedHand
//! } else {
//!     CursorStyle::OpenHand
//! }));
//! ```

pub use waterui_interaction::cursor::{Cursor, CursorStyle};
