#![cfg_attr(
    test,
    allow(
        clippy::float_cmp,
        reason = "tests assert exact animation/geometry values"
    )
)]
//! Core backend infrastructure for `WaterUI`.
//!
//! This crate provides shared infrastructure used by `WaterUI` backends:
//!
//! - [`ViewDispatcher`]: Type-based view dispatch for routing views to handlers
//! - [`frame_signals`]: the frame-economy trigger handle shared by self-drawn
//!   render loops (redraw / patch / structural rebuild requests)
//! - [`gesture`]: platform-agnostic gesture recognition state machines
//! - [`scroll`]: scroll offset/viewport math and handle registry
//! - [`animation`]: animated scalar sampling and animation key tracking
//! - [`input`]: platform-agnostic input event vocabulary
//! - [`time`]: monotonic clock abstraction valid across native and web targets
//!
//! Backends (GTK, hydrolysis, dew, etc.) build on this foundation while
//! implementing their own widget trees and rendering strategies.
//!
//! # Re-exports from `waterui-core`
//!
//! Common types are re-exported for convenience:
//! - Layout types: [`Size`], [`Point`], [`Rect`], [`ProposalSize`]
//! - Layout traits: [`SubView`], [`StretchAxis`], [`Layout`]
//! - View types: [`AnyView`], [`View`], [`Environment`]

#[cfg(feature = "widgets")]
pub mod animation;
pub mod dispatcher;
pub mod frame_signals;
#[cfg(feature = "gestures")]
pub mod gesture;
pub mod input;
pub mod scroll;
pub mod time;
#[cfg(feature = "widgets")]
pub mod widget;

pub use dispatcher::ViewDispatcher;
#[cfg(feature = "widgets")]
pub use widget::{Brush, DrawContext, WidgetTheme};

// Re-export common types from waterui-core
pub use waterui_core::{AnyView, Environment, Native, View};

// Re-export layout types from waterui-core::layout
pub use waterui_core::layout::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};
