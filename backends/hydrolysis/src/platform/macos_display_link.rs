//! Explicit `ProMotion` frame-rate voting for the winit runner on macOS.
//!
//! macOS only ramps a `ProMotion` panel up to its peak refresh rate when
//! something in the process declares a frame-rate range; wgpu's vsync-paced
//! present alone leaves the panel free to stay at a lower cadence. A
//! [`CADisplayLink`] created from the window's `NSView` (macOS 14+) carrying
//! the active [`FrameRate`] vote is that declaration.
//!
//! The link never drives rendering — winit's redraw path handles pacing. It
//! stays unpaused with the clamped frame-rate vote while the window votes
//! for continuous frames, and pauses itself as soon as the vote becomes
//! `None` so an idle window costs no display-link wakeups.

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSView;
use objc2_foundation::{NSObject, NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use waterui_frame_rate::FrameRate;
use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::window::Window as NativeWindow;

/// A per-window frame-rate vote declaration (see the module docs).
pub(super) struct FrameRateDemandLink {
    link: Retained<CADisplayLink>,
    _target: Retained<DemandLinkTarget>,
    applied: Cell<Option<(f32, f32, f32)>>,
}

impl core::fmt::Debug for FrameRateDemandLink {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FrameRateDemandLink")
            .field("paused", &self.link.isPaused())
            .field("applied", &self.applied.get())
            .finish_non_exhaustive()
    }
}

struct DemandLinkTargetIvars;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "WuiHydrolysisFrameRateDemand"]
    #[thread_kind = MainThreadOnly]
    #[ivars = DemandLinkTargetIvars]
    struct DemandLinkTarget;

    unsafe impl NSObjectProtocol for DemandLinkTarget {}

    impl DemandLinkTarget {
        /// Display-link callback; rendering is driven by winit, so this only
        /// keeps the `CADisplayLink` attached while unpaused.
        #[unsafe(method(step:))]
        fn step(&self, _link: &CADisplayLink) {}
    }
);

impl DemandLinkTarget {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DemandLinkTargetIvars);
        // SAFETY: `msg_send!` to `super.init` is the designated superclass
        // initializer for a `define_class!` type, and the `-> Retained<Self>`
        // signature is the one objc2 expects here.
        unsafe { msg_send![super(this), init] }
    }
}

/// Returns the window's `NSScreen::maximumFramesPerSecond` when available.
pub(super) fn screen_peak_fps(window: &NativeWindow) -> Option<f32> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit hands out the window's live `NSView` pointer, and the borrow
    // does not outlive the window handle it came from.
    let view = unsafe { appkit.ns_view.cast::<NSView>().as_ref() };
    let ns_window = view.window()?;
    let screen = ns_window.screen()?;
    let max_fps = screen.maximumFramesPerSecond();
    u16::try_from(max_fps)
        .ok()
        .filter(|&fps| fps > 0)
        .map(f32::from)
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
        link.setPaused(true);
        // SAFETY: main-thread message send to the link created just above, registering
        // it with the main run loop it will fire on.
        unsafe { link.addToRunLoop_forMode(&NSRunLoop::mainRunLoop(), NSRunLoopCommonModes) };
        Some(Self {
            link,
            _target: target,
            applied: Cell::new(None),
        })
    }

    /// Updates the `CADisplayLink` frame-rate vote for this window, clamping to
    /// `screen_peak_fps` with no 60Hz ceiling, or pauses it when idle (`None`).
    pub(super) fn set_vote(&self, vote: Option<FrameRate>, screen_peak_fps: f32) {
        let next = vote.map(|rate| {
            let clamped = rate.clamped(screen_peak_fps);
            (clamped.minimum(), clamped.maximum(), clamped.preferred())
        });
        if self.applied.get() == next {
            return;
        }
        self.applied.set(next);
        match next {
            Some((min, max, preferred)) => {
                self.link
                    .setPreferredFrameRateRange(CAFrameRateRange::new(min, max, preferred));
                if self.link.isPaused() {
                    self.link.setPaused(false);
                }
            }
            None => {
                if !self.link.isPaused() {
                    self.link.setPaused(true);
                }
            }
        }
    }
}

impl Drop for FrameRateDemandLink {
    fn drop(&mut self) {
        self.link.invalidate();
    }
}
