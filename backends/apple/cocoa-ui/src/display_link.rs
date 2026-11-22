//! A per-view frame clock.
//!
//! [`FrameClock`] drives a redraw tick for one view: while the view's window
//! has a screen it runs on a `CADisplayLink` at that display's maximum rate.
//! A window with no screen disarms the link — the view is not on any display,
//! so there is no native frame cadence to tick against — while leaving the
//! clock's request active so a later attachment re-arms it. A windowless view
//! stops the clock entirely.
//!
//! # Safety
//!
//! The `unsafe` here defines the `NSObject` action target the display link
//! talks to, on the kit thread contract: `CADisplayLink` and `NSWindow` are
//! main-thread objects, the link's selector is reached only on the main
//! thread, and the clock never outlives the view that owns it.

use std::cell::{Cell, RefCell};
use std::fmt;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol};
use objc2::sel;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSRunLoop, NSRunLoopCommonModes};
use objc2_quartz_core::{CADisplayLink, CAFrameRateRange};
use waterui_frame_rate::FrameRate;

use crate::PlatformView;
use crate::callback::guarded;

/// The ivars of a [`ClockTarget`].
struct ClockTargetIvars {
    on_frame: RefCell<Option<Rc<dyn Fn()>>>,
    /// The ticking link's `targetTimestamp` while the frame callback runs
    /// — the exact presentation timestamp the current callback is
    /// producing, shared with the owning [`FrameClock`].
    current_target: Rc<Cell<Option<f64>>>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements; the class holds one
    // main-thread closure and does not implement `Drop` — the clock clears
    // the closure before the target is released.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiFrameClockTarget"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ClockTargetIvars]
    /// The `CADisplayLink` action target of a [`FrameClock`].
    struct ClockTarget;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for ClockTarget {}

    impl ClockTarget {
        // SAFETY: `tick:` is the no-parameter action signature a
        // `CADisplayLink` posts; it fires only on the run loop the link was
        // added to, which is the main one.
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            let handler = self.ivars().on_frame.borrow().clone();
            if let Some(handler) = handler {
                // The exact frame this tick is producing, readable through
                // `FrameClock::current_target_timestamp` for the callback's
                // duration — never an approximation from another clock. The
                // cell is cloned out before the callback runs: a callback
                // that stops the clock drops `state.target` — possibly this
                // object's last strong ref — so `self` is not touched after.
                let current_target = self.ivars().current_target.clone();
                current_target.set(Some(link.targetTimestamp()));
                guarded("display link frame", move || handler());
                current_target.set(None);
            }
        }
    }
);

impl ClockTarget {
    /// A target that posts its frame callback to `on_frame`, recording the
    /// ticking link's target timestamp into `current_target` for the
    /// callback's duration.
    fn new(
        mtm: MainThreadMarker,
        on_frame: Rc<dyn Fn()>,
        current_target: Rc<Cell<Option<f64>>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClockTargetIvars {
            on_frame: RefCell::new(Some(on_frame)),
            current_target,
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this
    }
}

/// The armed display link and its target.
struct State {
    /// The display link driving the clock; `None` while nothing is armed.
    link: Option<Retained<CADisplayLink>>,
    /// The selector target, retained for the link's lifetime.
    target: Option<Retained<ClockTarget>>,
}

/// A redraw clock for one view: ticks at the window screen's maximum refresh
/// rate while the window is on a screen, and stays disarmed otherwise.
///
/// All methods are main-thread only. The closure `on_frame` is invoked once
/// per tick, on the main thread.
pub struct FrameClock {
    mtm: MainThreadMarker,
    on_frame: Rc<dyn Fn()>,
    active: Cell<bool>,
    vote: Cell<FrameRate>,
    screen_peak: Cell<Option<f32>>,
    /// Whether the clock holds its link paused: armed and registered, but
    /// not ticking. A link built while paused starts paused.
    paused: Cell<bool>,
    /// The exact `targetTimestamp` of the tick whose callback is running,
    /// `None` between ticks — the clock's own record of the current frame.
    current_target: Rc<Cell<Option<f64>>>,
    state: RefCell<State>,
}

impl fmt::Debug for FrameClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameClock")
            .field("active", &self.active.get())
            .field("vote", &self.vote.get())
            .field("paused", &self.paused.get())
            .field("armed", &self.state.borrow().link.is_some())
            .finish_non_exhaustive()
    }
}

/// Converts a `FrameRate` vote clamped against `peak_fps` into Core Animation's
/// `(minimum, maximum, preferred)` `CAFrameRateRange`.
pub(crate) fn ca_frame_rate_range(vote: FrameRate, peak_fps: f32) -> CAFrameRateRange {
    let clamped = vote.clamped(peak_fps);
    CAFrameRateRange::new(clamped.minimum(), clamped.maximum(), clamped.preferred())
}

impl FrameClock {
    /// A clock that posts `on_frame` once per tick while running.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, on_frame: impl Fn() + 'static) -> Self {
        Self {
            mtm,
            on_frame: Rc::new(on_frame),
            active: Cell::new(false),
            vote: Cell::new(FrameRate::FAST),
            screen_peak: Cell::new(None),
            paused: Cell::new(false),
            current_target: Rc::new(Cell::new(None)),
            state: RefCell::new(State {
                link: None,
                target: None,
            }),
        }
    }

    /// Updates the clock's refresh-rate vote, immediately updating any armed
    /// `CADisplayLink` against the current screen's peak rate.
    pub fn set_frame_rate(&self, vote: FrameRate) {
        if self.vote.get() == vote {
            return;
        }
        self.vote.set(vote);
        if let (Some(link), Some(peak)) = (&self.state.borrow().link, self.screen_peak.get()) {
            link.setPreferredFrameRateRange(ca_frame_rate_range(vote, peak));
        }
    }

    /// Returns the armed display link's `(minimum, maximum, preferred)` range,
    /// or `None` while disarmed.
    #[must_use]
    pub fn preferred_frame_rate_range(&self) -> Option<(f32, f32, f32)> {
        self.state.borrow().link.as_ref().map(|link| {
            let range = link.preferredFrameRateRange();
            (range.minimum, range.maximum, range.preferred)
        })
    }

    /// (Re)starts the clock against `view`'s current window and screen.
    ///
    /// A windowless view stops the clock; a window with no screen leaves the
    /// clock disarmed until a later [`FrameClock::reselect`] finds one.
    pub fn start(&self, view: &PlatformView) {
        self.active.set(true);
        self.reselect(view);
    }

    /// Stops the clock: the display link invalidates.
    pub fn stop(&self) {
        self.active.set(false);
        self.disarm();
    }

    /// Pauses (`true`) or resumes (`false`) the ticking without disarming:
    /// a paused link posts no frames and costs nothing, and resuming picks
    /// the cadence back up with no re-selection of the window or screen.
    /// The setting survives [`FrameClock::reselect`] and [`FrameClock::start`].
    pub fn set_paused(&self, paused: bool) {
        self.paused.set(paused);
        if let Some(link) = self.state.borrow().link.as_ref() {
            link.setPaused(paused);
        }
    }

    /// Whether the clock is held paused.
    #[must_use]
    pub const fn is_paused(&self) -> bool {
        self.paused.get()
    }

    /// Whether a display link is armed.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.borrow().link.is_some()
    }

    /// The `CACurrentMediaTime` target timestamp of the tick whose frame
    /// callback is currently running — `Some` only for that callback's
    /// duration, `None` anywhere else. This is the exact presentation time
    /// the current frame targets; there is deliberately no approximate
    /// now/window accessor, so callers outside the callback must use the
    /// shared presentation anchor's own capture time instead.
    #[must_use]
    pub fn current_target_timestamp(&self) -> Option<f64> {
        self.current_target.get()
    }

    /// Re-picks the clock for `view`'s current attachment.
    pub fn reselect(&self, view: &PlatformView) {
        if !self.active.get() {
            return;
        }
        let window = crate::view::window(view);
        let Some(window) = window else {
            self.stop();
            return;
        };
        let screen = window_screen(&window);
        let Some(screen) = screen else {
            // No display drives the view: disarm, but keep the request
            // active so re-attaching to a screen re-arms the link.
            self.disarm();
            return;
        };
        let link = self.make_link(&window, &screen);
        {
            let mut state = self.state.borrow_mut();
            if let Some(previous) = state.link.take() {
                previous.invalidate();
            }
            state.link = Some(link);
        }
    }

    /// Builds the display link against `screen`'s maximum frame rate.
    fn make_link(
        &self,
        window: &PlatformWindow,
        screen: &PlatformScreen,
    ) -> Retained<CADisplayLink> {
        let target = ClockTarget::new(self.mtm, self.on_frame.clone(), self.current_target.clone());
        // SAFETY: `displayLinkWithTarget:selector:` retains the pair until
        // the link invalidates; the ivars own the target.
        let link: Retained<CADisplayLink> = unsafe {
            #[cfg(target_os = "macos")]
            {
                msg_send![window, displayLinkWithTarget:&*target, selector:sel!(tick:)]
            }
            #[cfg(target_os = "ios")]
            {
                let _ = window;
                msg_send![objc2::class!(CADisplayLink), displayLinkWithTarget:&*target, selector:sel!(tick:)]
            }
        };
        let maximum =
            f32::from(u16::try_from(screen.maximumFramesPerSecond().max(1)).unwrap_or(u16::MAX));
        self.screen_peak.set(Some(maximum));
        link.setPaused(self.paused.get());
        link.setPreferredFrameRateRange(ca_frame_rate_range(self.vote.get(), maximum));
        // SAFETY: `addToRunLoop:forMode:` schedules the live link on this
        // thread's run loop; `NSRunLoopCommonModes` is a system constant.
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::currentRunLoop(), NSRunLoopCommonModes);
        }
        self.state.borrow_mut().target = Some(target);
        link
    }

    /// Drops whatever link is armed.
    fn disarm(&self) {
        self.screen_peak.set(None);
        let mut state = self.state.borrow_mut();
        if let Some(link) = state.link.take() {
            link.invalidate();
        }
        state.target = None;
    }
}

impl Drop for FrameClock {
    fn drop(&mut self) {
        self.disarm();
    }
}

#[cfg(target_os = "macos")]
type PlatformWindow = objc2_app_kit::NSWindow;
#[cfg(target_os = "macos")]
type PlatformScreen = objc2_app_kit::NSScreen;
#[cfg(target_os = "ios")]
type PlatformWindow = objc2_ui_kit::UIWindow;
#[cfg(target_os = "ios")]
type PlatformScreen = objc2_ui_kit::UIScreen;

/// The screen driving `window`, when the window is on one.
#[allow(clippy::unnecessary_wraps)]
fn window_screen(window: &PlatformWindow) -> Option<Retained<PlatformScreen>> {
    #[cfg(target_os = "macos")]
    {
        window.screen()
    }
    #[cfg(target_os = "ios")]
    {
        Some(window.screen())
    }
}

#[cfg(test)]
mod tests {
    use super::ca_frame_rate_range;
    use waterui_frame_rate::FrameRate;

    #[test]
    fn fast_vote_keeps_full_promotion_band_without_60hz_floor() {
        let range = ca_frame_rate_range(FrameRate::FAST, 120.0);
        assert_eq!(
            (range.minimum, range.maximum, range.preferred),
            (80.0, 120.0, 120.0)
        );
    }

    #[test]
    fn slow_and_fixed_votes_preserve_classified_bounds() {
        let slow = ca_frame_rate_range(FrameRate::SLOW, 120.0);
        assert_eq!(
            (slow.minimum, slow.maximum, slow.preferred),
            (30.0, 60.0, 60.0)
        );

        let cinema = ca_frame_rate_range(FrameRate::fixed(24.0), 120.0);
        assert_eq!(
            (cinema.minimum, cinema.maximum, cinema.preferred),
            (24.0, 24.0, 24.0)
        );

        let sixty = ca_frame_rate_range(FrameRate::FAST, 60.0);
        assert_eq!(
            (sixty.minimum, sixty.maximum, sixty.preferred),
            (60.0, 60.0, 60.0)
        );
    }
}
