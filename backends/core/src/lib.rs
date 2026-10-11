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
//! Backends build on this foundation while implementing their own widget
//! trees and rendering strategies.
//!
//! # The main-loop executor
//!
//! Whoever owns the main loop supplies the `LocalExecutor`. Every host has an
//! executor bound to its loop: winit's `WinitMainThreadExecutor`, GTK's
//! `GtkMainThreadExecutor` (through `glib::idle_add_local_once`), the headless
//! `HeadlessMainThreadExecutor`, and the embedded host's executor with its
//! per-frame tick. Hand that executor to `try_init_local_executor`, never
//! `native_executor::NativeExecutor`. On non-Apple targets `NativeExecutor`
//! delegates to a polyfill whose `spawn_main_local` asserts that it runs on
//! the thread `start_main_executor` registered. That entry point blocks and
//! never returns, so a host that owns its loop must not call it: it would
//! declare an unrelated thread the main thread, while `MainThreadBound`,
//! layout and the GPU surface all live on the loop thread. The mistake is
//! made at install time but panics only at the first `spawn_local`, so check
//! it whenever a new host or test harness is added. `NativeExecutor` is still
//! correct for `try_init_global_executor`, which needs no main-thread
//! affinity.
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
#[cfg(feature = "overlay")]
pub mod overlay;
pub mod scroll;
pub mod time;
#[cfg(feature = "widgets")]
pub mod widget;

pub use dispatcher::ViewDispatcher;
#[cfg(feature = "widgets")]
pub use widget::WidgetTheme;

// Re-export common types from waterui-core
pub use waterui_core::{AnyView, Environment, Native, View};

// Re-export layout types from waterui-core::layout
pub use waterui_core::layout::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};
