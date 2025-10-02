#![no_std]
//! Layout building blocks for `WaterUI`.
//!
//! This crate bridges the declarative [`View`](waterui_core::View) system with
//! the imperative, backend-driven layout pass. It contains:
//!
//! - the low-level [`Layout`](crate::Layout) trait and its geometry helpers,
//! - reusable containers such as [`spacer()`], [`padding::Padding`], and stacks,
//! - thin wrappers (for example [`scroll()`]) that signal backend-specific
//!   behaviour.
//!
//! High-level usage mirrors other `WaterUI` crates:
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
//! }
//! ```
//!
//! For a broader tour see the crate README.

extern crate alloc;

mod core;
pub use core::*;

pub mod spacer;
pub use spacer::{Spacer, spacer};
pub mod stack;

pub mod scroll;
pub use scroll::{ScrollView, scroll};
pub mod frame;

pub mod container;

pub use container::Container;

pub mod grid;
pub mod padding;
