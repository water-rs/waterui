#![no_std]
//! Layout building blocks for `WaterUI`.
//!
//! This crate bridges the declarative [`View`](waterui_core::View) system with
//! the imperative, backend-driven layout pass. It contains:
//!
//! - the low-level [`Layout`] trait and its geometry helpers,
//! - reusable containers such as [`spacer()`], [`padding::Padding`], and stacks,
//! - thin wrappers (for example [`scroll()`]) that signal backend-specific
//!   behaviour.
//!
//! # Logical Pixels (Points)
//!
//! All layout values use **logical pixels** (points/dp) - the same unit as design
//! tools like Figma, Sketch, and Adobe XD. Native backends handle conversion to
//! physical pixels based on screen density:
//!
//! - iOS/macOS: Uses points natively
//! - Android: Converts dp → pixels via `displayMetrics.density`
//!
//! This ensures `spacing(8.0)` or `width(100.0)` renders at the same physical
//! size across all platforms, matching your design specifications exactly.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui_layout::{stack, spacer};
//! use waterui_text::text;
//!
//! pub fn toolbar() -> impl waterui_core::View {
//!     stack::hstack((
//!         text("WaterUI"),
//!         spacer(),
//!         stack::vstack((text("Docs"), text("Blog"))),
//!     ))
//!     .spacing(8.0)  // 8pt spacing - same as Figma/Sketch
//! }
//! ```
//!
//! For a broader tour see the crate README.

extern crate alloc;

pub use waterui_core::layout::*;

pub mod spacer;
pub use spacer::{Spacer, spacer};
pub mod stack;

pub mod scroll;
pub use scroll::{ScrollView, scroll};
pub mod frame;

pub mod container;

pub use container::LazyContainer;

pub mod grid;
pub mod overlay;
pub mod padding;
pub mod safe_area;

pub use overlay::{Overlay, OverlayLayout, overlay};
pub use safe_area::{EdgeSet, IgnoreSafeArea};

#[cfg(test)]
mod tests;
