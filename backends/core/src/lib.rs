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
//! - [`dynamic`]: backend receiver protocol ([`DynamicNode`]) for `Dynamic` views
//! - [`frame_signals`]: the frame-economy trigger handle shared by self-drawn
//!   render loops (redraw / patch / structural rebuild requests)
//! - [`gesture`]: platform-agnostic gesture recognition state machines
//! - [`scroll`]: scroll offset/viewport math and handle registry
//! - `overlay` (feature `overlay`): anchored-overlay window-space placement
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

extern crate alloc;

use alloc::rc::Rc;
use core::fmt::{Debug, Formatter, Result as FmtResult};

use waterui::accessibility::AccessibilityRole;
use waterui_core::{Computed, Str};

pub mod child;
pub mod dynamic;
pub mod frame_signals;
#[cfg(feature = "gestures")]
pub mod gesture;
pub mod hooked;
pub mod input;
#[cfg(feature = "log")]
pub mod log;
pub mod native_backend;
#[cfg(feature = "overlay")]
pub mod overlay;
pub mod scroll;
pub mod time;

pub use child::{Child, ChildView, IntoChild, NativeBackendFamily};
pub use dynamic::DynamicNode;
pub use hooked::{Hooked, HookedFamily};
pub use native_backend::{NativeBackend, Realize, report_unprovided};
pub use waterui_meta::mark_reached;

/// Declared accessibility semantics for a backend-provided [`NativeView`](waterui_core::NativeView).
///
/// Extracted through
/// [`NativeView::realize_provided`](waterui_core::NativeView::realize_provided)
/// by semantic runtimes that build an accessibility tree without mounting a
/// platform view.
#[derive(Default, Clone)]
pub struct ProvidedSemantics {
    role: Option<AccessibilityRole>,
    label: Option<Computed<Str>>,
    on_activate: Option<Rc<dyn Fn()>>,
}

impl Debug for ProvidedSemantics {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        f.debug_struct("ProvidedSemantics")
            .field("role", &self.role)
            .field("label", &self.label)
            .field("on_activate", &self.on_activate.is_some())
            .finish()
    }
}

impl ProvidedSemantics {
    /// Creates empty semantics with no role, label, or activation handler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The component's declared default accessibility role, if any.
    #[must_use]
    pub const fn role(&self) -> Option<&AccessibilityRole> {
        self.role.as_ref()
    }

    /// The component's declared default reactive accessibility label, if any.
    #[must_use]
    pub const fn label(&self) -> Option<&Computed<Str>> {
        self.label.as_ref()
    }

    /// The component's semantic activation handler, if any.
    #[must_use]
    pub fn on_activate(&self) -> Option<&Rc<dyn Fn()>> {
        self.on_activate.as_ref()
    }

    /// Sets the component's default accessibility role.
    pub const fn set_role(&mut self, role: Option<AccessibilityRole>) {
        self.role = role;
    }

    /// Sets the component's default reactive accessibility label.
    pub fn set_label(&mut self, label: Option<Computed<Str>>) {
        self.label = label;
    }

    /// Sets the component's semantic activation handler.
    pub fn set_on_activate(&mut self, on_activate: Option<Rc<dyn Fn()>>) {
        self.on_activate = on_activate;
    }

    /// Sets the component's default accessibility role in builder style.
    #[must_use]
    pub const fn with_role(mut self, role: AccessibilityRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Sets the component's default reactive accessibility label in builder style.
    #[must_use]
    pub fn with_label(mut self, label: Computed<Str>) -> Self {
        self.label = Some(label);
        self
    }

    /// Sets the component's semantic activation handler in builder style.
    #[must_use]
    pub fn with_on_activate(mut self, on_activate: impl Fn() + 'static) -> Self {
        self.on_activate = Some(Rc::new(on_activate));
        self
    }
}

// Re-export common types from waterui-core
pub use waterui_core::{AnyView, Environment, Native, View};

// Re-export layout types from waterui-core::layout
pub use waterui_core::layout::{Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView};

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use core::cell::Cell;

    use super::ProvidedSemantics;
    use waterui::accessibility::AccessibilityRole;
    use waterui_core::{Computed, Str};

    #[test]
    fn provided_semantics_builders_setters_and_getters_round_trip() {
        let calls = Rc::new(Cell::new(0));
        let calls_for_cb = Rc::clone(&calls);

        let mut semantics = ProvidedSemantics::new()
            .with_role(AccessibilityRole::Button)
            .with_label(Computed::constant(Str::from_static("Play")))
            .with_on_activate(move || {
                calls_for_cb.set(calls_for_cb.get() + 1);
            });

        assert_eq!(semantics.role(), Some(&AccessibilityRole::Button));
        assert!(semantics.label().is_some());
        (semantics.on_activate().expect("activation handler"))();
        assert_eq!(calls.get(), 1);

        semantics.set_role(Some(AccessibilityRole::Section));
        semantics.set_label(None);
        semantics.set_on_activate(None);
        assert_eq!(semantics.role(), Some(&AccessibilityRole::Section));
        assert!(semantics.label().is_none());
        assert!(semantics.on_activate().is_none());
    }
}
