//! The display driver every transition clock runs on (§1.4).

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::cell::Cell;
use std::time::Duration;

use cocoa_ui::PlatformView;
use cocoa_ui::display_link::FrameClock;
use waterui_backend_core::Environment;

/// Advances one transition on a view's display link.
///
/// `tick` runs once per display frame with the delta between consecutive
/// [`FrameClock::current_target_timestamp`] values — the first tick gets
/// `Duration::ZERO`. No `Instant::now`, no sleeps: a paused link stops the
/// transition instead of warping it.
pub struct TransitionDriver {
    inner: Rc<Inner>,
}

/// The clock and its tick state behind one `Rc`, so the frame callback
/// reaches the clock through a `Weak` and the driver drops cleanly.
struct Inner {
    /// The display link: armed while `start`'s view has a window on a
    /// screen, disarmed otherwise.
    clock: FrameClock,
    /// Where each frame's instant is read.
    timeline: Timeline,
    /// The last tick's instant — the delta's base.
    last_target: Cell<Option<f64>>,
    /// The per-frame advance the owner installed.
    tick: Box<dyn Fn(Duration)>,
}

impl core::fmt::Debug for TransitionDriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransitionDriver").finish_non_exhaustive()
    }
}

/// The timeline a transition's frames are measured on: the display
/// link's target timestamps, or under the native suite a
/// [`ManualTransitionClock`] the environment carries.
#[derive(Clone, Debug)]
pub struct Timeline {
    /// The test's clock, when the environment installed one.
    #[cfg(feature = "native-test")]
    manual: Option<ManualTransitionClock>,
}

impl Timeline {
    /// The timeline transitions rendered under `env` run on.
    #[cfg_attr(
        not(feature = "native-test"),
        expect(
            clippy::missing_const_for_fn,
            reason = "the native-test build reads the environment"
        )
    )]
    pub fn of(env: &Environment) -> Self {
        #[cfg(not(feature = "native-test"))]
        let _ = env; // only the native suite installs a timeline of its own
        Self {
            #[cfg(feature = "native-test")]
            manual: env.get::<ManualTransitionClock>().cloned(),
        }
    }

    /// The instant a driver starting now measures from — `None` for the
    /// display link, whose first frame is the base.
    #[cfg_attr(
        not(feature = "native-test"),
        expect(
            clippy::missing_const_for_fn,
            clippy::unused_self,
            reason = "the native-test build reads the manual clock"
        )
    )]
    fn origin(&self) -> Option<f64> {
        #[cfg(feature = "native-test")]
        if let Some(manual) = &self.manual {
            return Some(manual.now.get().as_secs_f64());
        }
        None
    }

    /// The instant of the frame `clock` is delivering.
    #[cfg_attr(
        not(feature = "native-test"),
        expect(
            clippy::unused_self,
            reason = "the native-test build reads the manual clock"
        )
    )]
    fn frame_instant(&self, clock: &FrameClock) -> f64 {
        #[cfg(feature = "native-test")]
        if let Some(manual) = &self.manual {
            return manual.now.get().as_secs_f64();
        }
        clock
            .current_target_timestamp()
            .expect("the frame callback carries its target timestamp")
    }
}

/// A transition clock under the native suite's hand.
///
/// Inserted into the environment a subtree renders under, it becomes the
/// timeline of every transition driver started beneath it: display
/// frames keep arriving — captures retry and frames republish on each —
/// but time stands still until [`advance`](Self::advance) moves it, so a
/// test observes a run in flight for as long as it needs to.
#[cfg(feature = "native-test")]
#[derive(Clone, Debug, Default)]
pub struct ManualTransitionClock {
    /// Time elapsed on this clock's timeline.
    now: Rc<Cell<Duration>>,
}

#[cfg(feature = "native-test")]
impl ManualTransitionClock {
    /// Moves time forward by `by`; each running driver's next display
    /// frame advances its transition by the distance moved.
    pub fn advance(&self, by: Duration) {
        self.now.set(self.now.get() + by);
    }
}

impl TransitionDriver {
    /// Starts ticking against `on`'s window: `tick(delta)` runs once per
    /// display frame, the first tick with `Duration::ZERO` unless
    /// `timeline` moved since the start.
    pub fn start(
        mtm: cocoa_ui::MainThreadMarker,
        on: &PlatformView,
        timeline: Timeline,
        tick: impl Fn(Duration) + 'static,
    ) -> Self {
        let inner = Rc::new_cyclic(|weak: &Weak<Inner>| Inner {
            clock: FrameClock::new(mtm, {
                let weak = weak.clone();
                move || {
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    let now = inner.timeline.frame_instant(&inner.clock);
                    let delta = inner
                        .last_target
                        .replace(Some(now))
                        .map_or(Duration::ZERO, |previous| {
                            Duration::from_secs_f64((now - previous).max(0.0))
                        });
                    (inner.tick)(delta);
                }
            }),
            last_target: Cell::new(timeline.origin()),
            timeline,
            tick: Box::new(tick),
        });
        inner.clock.start(on);
        Self { inner }
    }

    /// Re-arms the clock against `on`'s current window — the view moved
    /// to a window after a loss stopped the link.
    pub fn restart(&self, on: &PlatformView) {
        self.inner.clock.start(on);
    }

    /// Stops the clock: the display link invalidates.
    pub fn stop(&self) {
        self.inner.clock.stop();
    }
}
