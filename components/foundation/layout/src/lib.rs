#![no_std]
#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "tests assert exact layout geometry values"
    )
)]
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
//! ```rust
//! use waterui::prelude::*;
//!
//! pub fn toolbar() -> impl View {
//!     hstack((
//!         text("WaterUI"),
//!         spacer(),
//!         vstack((text("Docs"), text("Blog"))),
//!     ))
//!     .spacing(8.0) // 8pt spacing - same as Figma/Sketch
//! }
//! ```
//!
//! For a broader tour see the crate README.

extern crate alloc;

pub use waterui_core::layout::*;

mod collections;
mod containers;
mod modifiers;

pub use collections::{grid, scroll};
pub use containers::{
    absolute, aspect_ratio, collection_transition, container, divider, frame, spacer,
};
pub use modifiers::{alignment_guide, background, overlay, padding, safe_area};

pub use divider::Divider;
pub use spacer::{Spacer, spacer, spacer_min};
pub mod stack;

pub use grid::{Grid, GridRow, grid, row};
pub use scroll::{ScrollController, ScrollView, scroll, scroll_both, scroll_horizontal};

pub use alignment_guide::{HorizontalAlignmentGuide, VerticalAlignmentGuide};
pub use aspect_ratio::{AspectRatio, AspectRatioLayout, ContentMode, aspect_ratio};
pub use collection_transition::{CollectionTransition, collection_transition};
pub use container::LazyContainer;

pub use background::{BackgroundLayout, BackgroundView, background};
pub use overlay::{Overlay, OverlayLayout, overlay};
pub use safe_area::{EdgeSet, IgnoreSafeArea};

pub use absolute::{
    Absolute, AbsoluteLayout, PinConstraints, PositionExt, PositionTarget, PositionedChild,
    PositionedLayout, absolute,
};

#[cfg(test)]
mod tests;
