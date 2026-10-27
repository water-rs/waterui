//! The display driver every transition clock runs on (§1.4).

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::cell::Cell;
use std::time::Duration;

use cocoa_ui::PlatformView;
use cocoa_ui::display_link::FrameClock;

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
    /// The last tick's target timestamp — the delta's base.
    last_target: Cell<Option<f64>>,
    /// The per-frame advance the owner installed.
    tick: Box<dyn Fn(Duration)>,
}

impl core::fmt::Debug for TransitionDriver {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransitionDriver").finish_non_exhaustive()
    }
}

impl TransitionDriver {
    /// Starts ticking against `on`'s window: `tick(delta)` runs once per
    /// display frame, the first tick with `Duration::ZERO`.
    pub fn start(
        mtm: cocoa_ui::MainThreadMarker,
        on: &PlatformView,
        tick: impl Fn(Duration) + 'static,
    ) -> Self {
        let inner = Rc::new_cyclic(|weak: &Weak<Inner>| Inner {
            clock: FrameClock::new(mtm, {
                let weak = weak.clone();
                move || {
                    let Some(inner) = weak.upgrade() else {
                        return;
                    };
                    let now = inner
                        .clock
                        .current_target_timestamp()
                        .expect("the frame callback carries its target timestamp");
                    let delta = inner
                        .last_target
                        .replace(Some(now))
                        .map_or(Duration::ZERO, |previous| {
                            Duration::from_secs_f64((now - previous).max(0.0))
                        });
                    (inner.tick)(delta);
                }
            }),
            last_target: Cell::new(None),
            tick: Box::new(tick),
        });
        inner.clock.start(on);
        Self { inner }
    }

    /// Stops the clock: the display link invalidates.
    pub fn stop(&self) {
        self.inner.clock.stop();
    }
}
