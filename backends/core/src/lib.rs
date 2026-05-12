//! Core backend infrastructure for `WaterUI`.
//!
//! This crate provides shared infrastructure used by `WaterUI` backends:
//!
//! - [`ViewDispatcher`]: Type-based view dispatch for routing views to handlers
//!
//! Backends (GTK, hydrolysis, etc.) build on this foundation while implementing
//! their own widget trees and rendering strategies.
//!
//! # Re-exports from `waterui-core`
//!
//! Common types are re-exported for convenience:
//! - Layout types: [`Size`], [`Point`], [`Rect`], [`ProposalSize`]
//! - Layout traits: [`SubView`], [`StretchAxis`], [`Layout`]
//! - View types: [`AnyView`], [`View`], [`Environment`]

pub mod dispatcher;
pub mod widget;

pub use dispatcher::ViewDispatcher;
pub use widget::{Brush, DrawContext, WidgetTheme};

// Re-export common types from waterui-core
pub use waterui_core::{AnyView, Environment, Native, View};

// Re-export layout types from waterui-core::layout
pub use waterui_core::layout::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};
