//! Explicit ProMotion opt-in for the winit runner on macOS.
//!
//! macOS only ramps a ProMotion panel up to 120Hz when something in the
//! process declares a frame-rate demand; wgpu's vsync-paced present alone
//! leaves the panel free to idle at a lower cadence. A [`CADisplayLink`]
//! created from the window's `NSView` (macOS 14+) carrying
//! `preferredFrameRateRange(60..=120, preferred 120)` is that declaration.
//!
//! The link never drives rendering — winit's redraw path is untouched. It
//! stays unpaused while redraws are being requested (plus a short hold so
//! continuous animation keeps one uninterrupted demand window), and pauses
//! itself afterwards so an idle window costs no display-link wakeups.

use std::cell::Cell;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObject, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window as NativeWindow;

/// The declared frame-rate range: ProMotion panels run up to 120Hz; the
/// system clamps the range on displays with a lower maximum.
const FRAME_RATE_MIN: f32 = 60.0;
const FRAME_RATE_MAX: f32 = 120.0;

/// How long the demand is held past the most recent redraw request. Animation
/// frames request redraws continuously, so the demand window stays open for
/// the whole animation and closes shortly after the last frame.
const DEMAND_HOLD: Duration = Duration::from_millis(250);

/// A per-window high-refresh demand declaration (see the module docs).
pub(super) struct FrameRateDemandLink {
    link: Retained<CADisplayLink>,
    target: Retained<DemandLinkTarget>,
}

impl core::fmt::Debug for FrameRateDemandLink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameRateDemandLink")
            .field("paused", &self.link.isPaused())
            .finish_non_exhaustive()
    }
}

struct DemandLinkTargetIvars {
    demand_until: Cell<Instant>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WuiHydrolysisFrameRateDemand"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DemandLinkTargetIvars]
    struct DemandLinkTarget;

    unsafe impl NSObjectProtocol for DemandLinkTarget {}

    impl DemandLinkTarget {
        /// Per-refresh callback. Its only job is to close the demand window
        /// once redraw requests have stopped arriving; it never renders.
        #[unsafe(method(step:))]
        fn step(&self, link: &CADisplayLink) {
            if Instant::now() >= self.ivars().demand_until.get() {
                link.setPaused(true);
            }
        }
    }
);

impl DemandLinkTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DemandLinkTargetIvars {
            demand_until: Cell::new(Instant::now()),
        });
        // SAFETY: `msg_send!` to `super.init` is the designated superclass
        // initializer for a `define_class!` type, and the `-> Retained<Self>`
        // signature is the one objc2 expects here.
        unsafe { msg_send![super(this), init] }
    }
}

impl FrameRateDemandLink {
    /// Attaches a paused frame-rate demand link to the window's view.
    ///
    /// Returns `None` before macOS 14 (per-view display links do not exist
    /// there); the window then runs at whatever rate the system picks, as it
    /// did before this opt-in existed.
    pub(super) fn attach(window: &NativeWindow) -> Option<Self> {
        let mtm = MainThreadMarker::new()?;
        let _ = mtm;
        let handle = window.window_handle().ok()?;
        let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return None;
        };
        // The handle's view pointer is valid while the winit window lives;
        // the reference does not escape this call.
        // SAFETY: winit hands out the window's live `NSView` pointer, and the borrow
        // does not outlive the window handle it came from.
        let view = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
        if !view.respondsToSelector(sel!(displayLinkWithTarget:selector:)) {
            return None;
        }
        let target = DemandLinkTarget::new(mtm);
        let target_object: &AnyObject = &target;
        // SAFETY: the `respondsToSelector` check above proves this macOS version has
        // the selector; `target_object` is retained by `self` for the link's lifetime,
        // and `step:` is defined on it. Main thread, as `NSView` requires.
        let link = unsafe { view.displayLinkWithTarget_selector(target_object, sel!(step:)) };
        link.setPreferredFrameRateRange(CAFrameRateRange::new(
            FRAME_RATE_MIN,
            FRAME_RATE_MAX,
            FRAME_RATE_MAX,
        ));
        link.setPaused(true);
        // SAFETY: main-thread message send to the link created just above, registering
        // it with the main run loop it will fire on.
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        Some(Self { link, target })
    }

    /// Declares high-refresh demand for the next [`DEMAND_HOLD`]. Called with
    /// every redraw request, so the demand stays continuous while frames are
    /// being produced.
    pub(super) fn hold_demand(&self) {
        self.target
            .ivars()
            .demand_until
            .set(Instant::now() + DEMAND_HOLD);
        if self.link.isPaused() {
            self.link.setPaused(false);
        }
    }
}

impl Drop for FrameRateDemandLink {
    fn drop(&mut self) {
        self.link.invalidate();
    }
}
