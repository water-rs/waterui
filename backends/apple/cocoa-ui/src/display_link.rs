//! A per-view frame clock.
//!
//! [`FrameClock`] drives a redraw tick for one view: while the view's window
//! has a screen it runs on a `CADisplayLink` at that display's maximum rate;
//! when the window loses its screen — or on devices where a display link
//! cannot run — it degrades to re-arming itself off the main run loop, so a
//! GPU surface keeps ticking through screen changes and off-screen windows.
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

use crate::PlatformView;
use crate::callback::guarded;
use crate::main_queue::enqueue_local;

/// The ivars of a [`ClockTarget`].
struct ClockTargetIvars {
    on_frame: RefCell<Option<Rc<dyn Fn()>>>,
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
        fn tick(&self, _link: &CADisplayLink) {
            let handler = self.ivars().on_frame.borrow().clone();
            if let Some(handler) = handler {
                guarded("display link frame", move || handler());
            }
        }
    }
);

impl ClockTarget {
    /// A target that posts its frame callback to `on_frame`.
    fn new(mtm: MainThreadMarker, on_frame: Rc<dyn Fn()>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClockTargetIvars {
            on_frame: RefCell::new(Some(on_frame)),
        });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this
    }
}

/// Which clock the driver currently runs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Clock {
    /// A `CADisplayLink` on the window's screen.
    DisplayLink,
    /// A run-loop re-arm: no screen is driving the view.
    RunLoop,
}

/// The per-screen display link and its target.
struct State {
    /// Which clock is armed.
    clock: Option<Clock>,
    /// The display link, when `clock == DisplayLink`.
    link: Option<Retained<CADisplayLink>>,
    /// The selector target, retained for the link's lifetime.
    target: Option<Retained<ClockTarget>>,
}

/// A redraw clock for one view: ticks at the window screen's maximum refresh
/// rate, degrading to a run-loop re-arm when no screen drives the view.
///
/// All methods are main-thread only. The closure `on_frame` is invoked once
/// per tick, on the main thread.
pub struct FrameClock {
    mtm: MainThreadMarker,
    on_frame: Rc<dyn Fn()>,
    active: Cell<bool>,
    state: RefCell<State>,
}

impl fmt::Debug for FrameClock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FrameClock")
            .field("active", &self.active.get())
            .field("clock", &self.state.borrow().clock)
            .finish_non_exhaustive()
    }
}

impl FrameClock {
    /// A clock that posts `on_frame` once per tick while running.
    #[must_use]
    pub fn new(mtm: MainThreadMarker, on_frame: impl Fn() + 'static) -> Self {
        Self {
            mtm,
            on_frame: Rc::new(on_frame),
            active: Cell::new(false),
            state: RefCell::new(State {
                clock: None,
                link: None,
                target: None,
            }),
        }
    }

    /// (Re)starts the clock against `view`'s current window and screen.
    ///
    /// A window with no screen starts the run-loop clock; a windowless view
    /// stops the clock entirely.
    pub fn start(&self, view: &PlatformView) {
        self.active.set(true);
        self.reselect(view);
    }

    /// Stops the clock: the display link invalidates, the run-loop re-arm
    /// stops rescheduling itself.
    pub fn stop(&self) {
        self.active.set(false);
        self.disarm();
    }

    /// Whether the clock is armed.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.borrow().clock.is_some()
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
            self.arm_run_loop();
            return;
        };
        let link = self.make_link(&window, &screen);
        {
            let mut state = self.state.borrow_mut();
            if let Some(previous) = state.link.take() {
                previous.invalidate();
            }
            state.link = Some(link);
            state.clock = Some(Clock::DisplayLink);
        }
    }

    /// Builds the display link against `screen`'s maximum frame rate.
    fn make_link(
        &self,
        window: &PlatformWindow,
        screen: &PlatformScreen,
    ) -> Retained<CADisplayLink> {
        let target = ClockTarget::new(self.mtm, self.on_frame.clone());
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
        link.setPreferredFrameRateRange(CAFrameRateRange::new(maximum.min(60.0), maximum, maximum));
        // SAFETY: `addToRunLoop:forMode:` schedules the live link on this
        // thread's run loop; `NSRunLoopCommonModes` is a system constant.
        unsafe {
            link.addToRunLoop_forMode(&NSRunLoop::currentRunLoop(), NSRunLoopCommonModes);
        }
        self.state.borrow_mut().target = Some(target);
        link
    }

    /// Arms the run-loop clock: one async hop per tick, re-armed while it
    /// still owns the clock.
    fn arm_run_loop(&self) {
        {
            let mut state = self.state.borrow_mut();
            if let Some(link) = state.link.take() {
                link.invalidate();
            }
            state.clock = Some(Clock::RunLoop);
        }
        self.schedule_wake();
    }

    /// One wake of the run-loop clock.
    fn schedule_wake(&self) {
        let on_frame = self.on_frame.clone();
        let mtm = self.mtm;
        let state = std::ptr::from_ref::<Self>(self);
        enqueue_local(mtm, move |_| {
            // The clock's owner keeps it alive; the hop runs only while the
            // run-loop clock is still the armed one.
            // SAFETY: the hop is dropped with the main queue when the owner
            // stops owning the clock — the view stops the clock before it
            // releases it, so `state` stays valid for any scheduled hop.
            let clock = unsafe { &*state };
            if clock.state.borrow().clock == Some(Clock::RunLoop) {
                (on_frame)();
                clock.schedule_wake();
            }
        });
    }

    /// Drops whatever clock is armed.
    fn disarm(&self) {
        let mut state = self.state.borrow_mut();
        if let Some(link) = state.link.take() {
            link.invalidate();
        }
        state.clock = None;
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
