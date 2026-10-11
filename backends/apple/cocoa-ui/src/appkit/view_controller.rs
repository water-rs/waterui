//! View controllers.
//!
//! # Safety
//!
//! The `unsafe` here allocates and initializes an `NSViewController` — the
//! documented `init` initializer on the main thread.

use objc2::ClassType;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{NSView, NSViewController};

/// A view controller, created empty.
///
/// A controller in the window's controller chain is what controller-based
/// components need: `NSSplitViewItem` only reaches the titlebar when its
/// split view lives under an `NSViewController` the window knows.
#[derive(Debug)]
pub struct ViewController {
    controller: Retained<NSViewController>,
}

impl ViewController {
    /// An empty controller with no view.
    #[must_use]
    pub fn new(mtm: MainThreadMarker) -> Self {
        // SAFETY: see the module safety note.
        let _ = mtm;
        // SAFETY: `new` is `alloc` + `init`, `NSViewController`'s designated
        // no-argument initializer, safe to send on the main thread this
        // `mtm` proves we are on.
        let controller: Retained<NSViewController> =
            unsafe { objc2::msg_send![NSViewController::class(), new] };
        Self { controller }
    }

    /// Makes `view` the controller's view.
    pub fn set_view(&self, view: &NSView) {
        self.controller.setView(view);
    }

    /// The controller as `AppKit` sees it.
    pub(crate) fn native(&self) -> &NSViewController {
        &self.controller
    }
}
