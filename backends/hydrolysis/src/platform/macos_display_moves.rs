//! Display-move tracking for the engine-presented macOS window.
//!
//! `cherenkov::Surface` re-runs output negotiation — colour space, headroom,
//! the plane system's displays — only when its host announces a move
//! through `Surface::display_moved`. winit does not deliver a screen-change
//! event, so the announcement comes from the window's
//! `NSWindowDidChangeScreenNotification`, which fires for every move
//! including the scale-preserving ones winit's events miss. The observer
//! only sets a flag: the render frame reads and clears it and makes the
//! engine call itself, so `display_moved` is reported exactly once per
//! move on the thread that owns the surface.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, sel};
use objc2_app_kit::NSWindowDidChangeScreenNotification;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSObject};
use waterui_core::contract_violation;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window as NativeWindow;

struct DisplayMoveIvars {
    moved: Rc<Cell<bool>>,
    window: Arc<NativeWindow>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WuiHydrolysisDisplayMove"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DisplayMoveIvars]
    struct DisplayMoveObserver;

    unsafe impl NSObjectProtocol for DisplayMoveObserver {}

    impl DisplayMoveObserver {
        /// `NSWindowDidChangeScreenNotification`: the window crossed
        /// displays; the flag is consumed by the next render frame and
        /// winit is asked to redraw.
        #[unsafe(method(displayMoved:))]
        fn display_moved(&self, _notification: &NSNotification) {
            self.ivars().moved.set(true);
            self.ivars().window.request_redraw();
        }
    }
);

impl DisplayMoveObserver {
    fn new(
        mtm: MainThreadMarker,
        flag: Rc<Cell<bool>>,
        window: Arc<NativeWindow>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DisplayMoveIvars {
            moved: flag,
            window,
        });
        // SAFETY: `msg_send!` to `super.init` is the designated superclass
        // initializer for a `define_class!` type, and the `-> Retained<Self>`
        // signature is the one objc2 expects here.
        unsafe { objc2::msg_send![super(this), init] }
    }
}

/// The flag the observer writes and the render frame clears.
pub(super) type DisplayMoveFlag = Rc<Cell<bool>>;

/// Observes the window's screen-change notifications, recording each as a
/// set bit on `flag` and requesting a redraw.
pub(super) struct DisplayMoves {
    observer: Retained<DisplayMoveObserver>,
}

impl core::fmt::Debug for DisplayMoves {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DisplayMoves").finish_non_exhaustive()
    }
}

impl DisplayMoves {
    /// Attaches an observer to the window's
    /// `NSWindowDidChangeScreenNotification`.
    ///
    /// `None` when the window exposes no `NSWindow` to observe: the failure
    /// is reported and the window announces no display moves.
    pub(super) fn attach(window: Arc<NativeWindow>, flag: DisplayMoveFlag) -> Option<Self> {
        let mtm = MainThreadMarker::new().expect(
            "the winit event loop exists only on the main thread, and windows are created inside it",
        );
        let handle = match window.window_handle() {
            Ok(handle) => handle,
            Err(error) => {
                tracing::error!(
                    "hydrolysis display observer found no window handle: {error}; display moves are not announced"
                );
                return None;
            }
        };
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            contract_violation!(
                "hydrolysis display observer expected an AppKit raw window handle; display moves are not announced"
            );
            return None;
        };
        // SAFETY: winit hands out the window's live `NSView` pointer, and
        // the borrow does not outlive the window handle it came from.
        let view = unsafe { appkit.ns_view.cast::<objc2_app_kit::NSView>().as_ref() };
        let Some(ns_window) = view.window() else {
            contract_violation!(
                "hydrolysis display observer expected the AppKit view to have an NSWindow; display moves are not announced"
            );
            return None;
        };
        let observer = DisplayMoveObserver::new(mtm, flag, window);
        let observer_object: &AnyObject = &observer;
        let window_object: &AnyObject = &ns_window;
        // SAFETY: registering the observer on the notification's source
        // window; `displayMoved:` is defined on the class. The selector-based
        // notification center does not retain selector-based observers;
        // `DisplayMoves` retains it for the registration's lifetime and
        // removes it in `Drop`.
        unsafe {
            NSNotificationCenter::defaultCenter().addObserver_selector_name_object(
                observer_object,
                sel!(displayMoved:),
                Some(NSWindowDidChangeScreenNotification),
                Some(window_object),
            );
        }
        Some(Self { observer })
    }
}

impl Drop for DisplayMoves {
    fn drop(&mut self) {
        let observer_object: &AnyObject = &self.observer;
        // SAFETY: this is the observer `attach` registered; unregistering it
        // removes the notification registration before the retained observer
        // is dropped.
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(observer_object);
        }
    }
}
