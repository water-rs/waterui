//! Touch-drag scrolling: the slop claim, one-to-one tracking and the
//! release fling.
//!
//! A [`PointerKind::Touch`] press inside a scroll view still delivers its
//! press to the content under it. Once the contact moves farther than the
//! host's touch slop, the innermost enclosing scroll view able to scroll
//! along the movement's dominant axis claims the gesture — the content's
//! press is cancelled, never completed — and every later move scrolls the
//! view's [`ScrollHandle`] one to one. On release, a [`VelocityTracker`]
//! fit over the last samples starts a [`TouchFling`] that decelerates with
//! the platform's deceleration model; a new touch down stops the fling
//! where it is.
//!
//! What an edge does is the physics family's [`Overscroll`]: clamped
//! edges stop the drag and the fling on them; rubber-band edges let a drag
//! past them move with resistance and spring a release or a fling past them
//! back onto them, the offset held unclamped meanwhile — rendered and
//! reported where it is, as `UIScrollView`'s `contentOffset` is.
//!
//! The gesture's parameters are the host's [`TouchScrollConfig`], pushed
//! into the renderer by value from `PlatformWindow::touch_scroll_config` —
//! no platform values are invented here.
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
use super::scroll_physics::{DecayFling, SpringFling};
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform::{FlingDeceleration, Overscroll, TouchScrollConfig};
use std::collections::VecDeque;
use waterui_layout::scroll::Axis;

/// The trailing window the release velocity is fit over — the span
/// Android's `VelocityTracker` weighs its motion samples against.
const VELOCITY_WINDOW: Duration = Duration::from_millis(100);
/// The most move samples a tracker keeps — far more than a vsync-paced
/// stream produces inside the window.
const VELOCITY_MAX_SAMPLES: usize = 32;

/// The in-flight touch-drag scroll gesture — one at a time, for the single
/// active pointer the hit-test state tracks.
#[derive(Debug, Default)]
pub enum TouchScrollGesture {
    /// No touch sequence is driving a scroll.
    #[default]
    Idle,
    /// A touch is down; the first move past the touch slop decides whether
    /// a scroll view claims the sequence.
    Pending(TouchScrollPending),
    /// A scroll view claimed the drag: every move scrolls it one to one.
    Dragging(TouchScrollDrag),
}

/// A touch press still inside the slop, waiting on the claim check.
#[derive(Debug)]
pub struct TouchScrollPending {
    /// Where the contact went down, in window hit-test space — the claim
    /// tests displacement and the scroll candidates at this point.
    pub(crate) origin: kurbo::Point,
    /// The motion samples the release's fling is fit over.
    pub(crate) tracker: VelocityTracker,
    /// The fling this touch down stopped, still holding its claim: a drag
    /// of the same scroll view takes the offset from it where it stands —
    /// a caught bounce included — and anything else lets it go again from
    /// rest, so an offset it held past an edge springs back.
    pub(crate) caught: Option<TouchFling>,
}

/// A scroll view's claimed drag, scrolled one to one by later moves.
#[derive(Debug)]
pub struct TouchScrollDrag {
    /// The claimed scroll view's offset handle.
    pub(crate) handle: crate::scroll::ScrollHandle,
    /// The claim the gesture took on the offset when the drag was
    /// recognised: every move writes through it, the release hands it to
    /// the fling, and only a newer claim — never a `rebind` for rows
    /// measured mid-drag — takes the offset from it.
    pub(crate) claim: crate::scroll::GestureClaim,
    /// The cell that registered the claimed scroll target — mark
    /// attribution for the drag's offset writes.
    pub(crate) owner: std::rc::Weak<crate::renderer::NodeCell>,
    /// The point the last applied delta ended at, in window hit-test space.
    pub(crate) last: kurbo::Point,
    /// The motion samples the release's fling is fit over.
    pub(crate) tracker: VelocityTracker,
    /// What the claimed view's edges do to the drag, from the physics the
    /// claim was taken under.
    pub(crate) overscroll: Overscroll,
}

impl TouchScrollDrag {
    /// Moves the claimed offset by `(dx, dy)` — the finger's travel,
    /// negated — and returns whether it moved: clamped edges stop it,
    /// rubber-band edges let it past with their resistance. Refused once a
    /// newer claim owns the offset.
    pub(crate) fn scroll_by(&self, dx: f64, dy: f64) -> bool {
        match self.overscroll {
            Overscroll::Clamped => self.handle.apply_gesture_delta(&self.claim, dx, dy),
            Overscroll::RubberBand(band) => {
                let metrics = self.handle.metrics();
                self.handle.apply_gesture_offset(
                    &self.claim,
                    Some(band.drag(metrics.offset_x, dx, metrics.max_x, metrics.viewport_width)),
                    Some(band.drag(metrics.offset_y, dy, metrics.max_y, metrics.viewport_height)),
                )
            }
        }
    }
}

/// The release velocity a fling starts from: a least-squares slope over
/// the most recent [`VELOCITY_WINDOW`] of move samples, the same shape
/// Android's `VelocityTracker` computes for a touch's `computeCurrentVelocity`.
#[derive(Debug)]
pub struct VelocityTracker {
    samples: VecDeque<(Instant, kurbo::Point)>,
}

impl VelocityTracker {
    /// Starts the track with the pointer-down sample.
    pub(crate) fn new(at: Instant, origin: kurbo::Point) -> Self {
        let mut samples = VecDeque::with_capacity(VELOCITY_MAX_SAMPLES);
        samples.push_back((at, origin));
        Self { samples }
    }

    /// Records a move sample.
    pub(crate) fn record(&mut self, at: Instant, point: kurbo::Point) {
        if self.samples.len() == VELOCITY_MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back((at, point));
    }

    /// The fitted pointer velocity at `now`, in logical units per second —
    /// the regression slope of the samples still inside the trailing
    /// window, or zero when fewer than two samples or no elapsed time
    /// support a slope.
    pub(crate) fn velocity(&self, now: Instant) -> kurbo::Vec2 {
        let cutoff = now.checked_sub(VELOCITY_WINDOW);
        let mut count = 0_usize;
        let mut mean_t = 0.0;
        let mut mean_x = 0.0;
        let mut mean_y = 0.0;
        for &(at, point) in &self.samples {
            if cutoff.is_some_and(|cutoff| at < cutoff) {
                continue;
            }
            let t = -now.saturating_duration_since(at).as_secs_f64();
            count += 1;
            mean_t += t;
            mean_x += point.x;
            mean_y += point.y;
        }
        if count < 2 {
            return kurbo::Vec2::ZERO;
        }
        let count = crate::num_cast::usize_as_f64(count);
        mean_t /= count;
        mean_x /= count;
        mean_y /= count;
        let mut numerator_x = 0.0;
        let mut numerator_y = 0.0;
        let mut denominator = 0.0;
        for &(at, point) in &self.samples {
            if cutoff.is_some_and(|cutoff| at < cutoff) {
                continue;
            }
            let dt = -now.saturating_duration_since(at).as_secs_f64() - mean_t;
            numerator_x = dt.mul_add(point.x - mean_x, numerator_x);
            numerator_y = dt.mul_add(point.y - mean_y, numerator_y);
            denominator = dt.mul_add(dt, denominator);
        }
        if denominator < f64::EPSILON {
            return kurbo::Vec2::ZERO;
        }
        kurbo::Vec2::new(numerator_x / denominator, numerator_y / denominator)
    }
}

/// A fling running after the touch released: per-axis deceleration — and,
/// under rubber-band edges, the spring back onto an edge — driving the
/// claimed scroll view's handle until it comes to rest inside the content
/// bounds or a new touch down stops it.
#[derive(Debug)]
pub struct TouchFling {
    handle: crate::scroll::ScrollHandle,
    /// The drag's claim, carried into the fling — the ownership record: it
    /// keeps writing while it is still the newest claim, and its first
    /// refused write — a request, a jump, user input — ends it rather than
    /// overwriting its successor. A `rebind` — shifted by a membership
    /// anchor or for rows measured mid-gesture — never touches it.
    claim: crate::scroll::GestureClaim,
    /// What the edges do: clamped edges write through the clamping fling
    /// path, rubber-band edges through the unclamped one.
    overscroll: Overscroll,
    x: Option<AxisFling>,
    y: Option<AxisFling>,
}

/// One axis of a [`TouchFling`].
#[derive(Debug)]
enum AxisFling {
    /// Android `OverScroller`'s spline.
    Spline(SplineFling),
    /// `UIScrollView`'s decay, with its bounce at an edge it reaches.
    Decay(DecayFling),
    /// The return from a release past an edge, and — for a release moving
    /// toward the content — the fling it hands over to on the edge, from
    /// the moment it starts.
    Release {
        spring: SpringFling,
        then: Option<(Instant, Box<Self>)>,
    },
}

impl AxisFling {
    /// The axis's offset at `now` and whether it still moves.
    fn position(&self, now: Instant) -> (f64, bool) {
        match self {
            Self::Spline(fling) => fling.position(now),
            Self::Decay(fling) => fling.position(now),
            Self::Release { spring, then } => match then {
                Some((from, fling)) if now >= *from => fling.position(now),
                _ => spring.position(now),
            },
        }
    }
}

impl TouchFling {
    /// Starts a fling on `handle` from the fitted pointer velocity, in
    /// logical units per second, on the released drag's `claim`: the
    /// offset's velocity is the finger's negated, clamped to the host's
    /// maximum. Under rubber-band edges an axis released past an edge
    /// returns onto it whatever its speed, starting at that velocity — the
    /// finger's, as `UIScrollView` reports it — and, moving toward the
    /// content, flings on into it from the edge; otherwise an axis under
    /// the minimum — or already at the edge it heads toward, under clamped
    /// edges — does not fling. `None` when no axis moves or a newer claim
    /// already owns the offset.
    pub(crate) fn start(
        handle: crate::scroll::ScrollHandle,
        claim: crate::scroll::GestureClaim,
        finger_velocity: kurbo::Vec2,
        config: &TouchScrollConfig,
        at: Instant,
    ) -> Option<Self> {
        let metrics = handle.metrics();
        let max = f64::from(config.max_fling_velocity);
        let min = f64::from(config.min_fling_velocity);
        let physics = config.physics;
        let axis_fling = |velocity: f64, start: f64, extent: f64| {
            // The content offset moves opposite the finger: a finger flung
            // up pushes the offset down into the content.
            let offset_velocity = (-velocity).clamp(-max, max);
            // A fling inside the extents from `start` at `velocity`, from
            // `from`.
            let inside = |velocity: f64, start: f64, from: Instant| {
                let bounce = match physics.overscroll {
                    // An axis with nothing to scroll does not bounce — the
                    // band's `drag` documents why.
                    Overscroll::RubberBand(band) => (extent > 0.0).then_some(band.bounce),
                    Overscroll::Clamped => None,
                };
                match physics.deceleration {
                    FlingDeceleration::OverScrollerSpline(spline) => Some(SplineFling::new(
                        velocity,
                        start,
                        extent,
                        spline.physical_coeff,
                        spline.friction,
                        from,
                    ))
                    // A run whose clamped destination is already the offset —
                    // released at the edge it headed toward — is no fling.
                    .filter(|fling| (fling.final_offset - fling.start).abs() > SCROLL_END_EPSILON)
                    .map(AxisFling::Spline),
                    FlingDeceleration::ExponentialDecay(decay) => {
                        DecayFling::new(decay, velocity, start, extent, bounce, from)
                            .map(AxisFling::Decay)
                    }
                }
            };
            if let Overscroll::RubberBand(band) = physics.overscroll
                && !(0.0..=extent).contains(&start)
            {
                let edge = start.clamp(0.0, extent);
                let overscroll = start - edge;
                let then =
                    band.release
                        .handover(overscroll, offset_velocity)
                        .and_then(|handover| {
                            let from = at + Duration::from_secs_f64(handover.seconds);
                            inside(handover.velocity, edge, from)
                                .map(|fling| (from, Box::new(fling)))
                        });
                return Some(AxisFling::Release {
                    spring: SpringFling::new(edge, overscroll, offset_velocity, band.release, at),
                    then,
                });
            }
            if offset_velocity.abs() < min {
                return None;
            }
            inside(offset_velocity, start, at)
        };
        let x = matches!(handle.axis(), Axis::Horizontal | Axis::All)
            .then(|| axis_fling(finger_velocity.x, metrics.offset_x, metrics.max_x))
            .flatten();
        let y = matches!(handle.axis(), Axis::Vertical | Axis::All)
            .then(|| axis_fling(finger_velocity.y, metrics.offset_y, metrics.max_y))
            .flatten();
        ((x.is_some() || y.is_some()) && handle.begin_fling(&claim)).then_some(Self {
            handle,
            claim,
            overscroll: physics.overscroll,
            x,
            y,
        })
    }

    /// Releases the claim `handle` and `claim` name — a drag let go, or a
    /// fling a touch caught and let go again: the fling the release earns
    /// under `config`, or, when it earns none, the gesture's end, which
    /// clamps an offset the claim held past an edge.
    pub(crate) fn release(
        handle: &crate::scroll::ScrollHandle,
        claim: &crate::scroll::GestureClaim,
        finger_velocity: kurbo::Vec2,
        config: &TouchScrollConfig,
        at: Instant,
    ) -> TouchRelease {
        Self::start(handle.clone(), claim.clone(), finger_velocity, config, at).map_or_else(
            || TouchRelease::Ended {
                moved: handle.end_gesture(claim),
            },
            |fling| TouchRelease::Fling(Box::new(fling)),
        )
    }

    /// Lets this caught fling go again from rest at `at`: see
    /// [`Self::release`].
    pub(crate) fn let_go(&self, config: &TouchScrollConfig, at: Instant) -> TouchRelease {
        Self::release(&self.handle, &self.claim, kurbo::Vec2::ZERO, config, at)
    }

    /// The claimed scroll view's offset handle — the pump needs it to
    /// attribute the tick's write to the `ScrollTarget` owner.
    pub(crate) const fn handle(&self) -> &crate::scroll::ScrollHandle {
        &self.handle
    }

    /// Advances the fling to `now` and applies its offset through the
    /// handle.
    pub(crate) fn tick(&self, now: Instant) -> TouchFlingTick {
        // A programmatic request — jump or animated — or any user input
        // claims the offset past this fling's claim: the fling ends on the
        // spot instead of writing over whatever replaced it. Ownership is
        // asked, never inferred from a write that moved nothing — the tick
        // on the release's own frame samples the release offset itself.
        if !self.handle.holds(&self.claim) {
            return TouchFlingTick {
                changed: false,
                running: false,
            };
        }
        // Only the axes with a run are sampled: an axis without one keeps
        // its offset, so a membership shift is never applied to it twice.
        let x = self.x.as_ref().map(|fling| fling.position(now));
        let y = self.y.as_ref().map(|fling| fling.position(now));
        let (offset_x, offset_y) = (x.map(|(offset, _)| offset), y.map(|(offset, _)| offset));
        let changed = match self.overscroll {
            Overscroll::Clamped => self
                .handle
                .apply_fling_offset(&self.claim, offset_x, offset_y),
            Overscroll::RubberBand(_) => {
                self.handle
                    .apply_overscroll_fling_offset(&self.claim, offset_x, offset_y)
            }
        };
        let active = |axis: Option<(f64, bool)>| axis.is_some_and(|(_, active)| active);
        let running = active(x) || active(y);
        // A fling that ends ends its gesture: an offset it still holds past
        // an edge — the content changed under a bounce — clamps into the
        // extents live now.
        let ended = !running && self.handle.end_gesture(&self.claim);
        TouchFlingTick {
            changed: changed || ended,
            running,
        }
    }
}

/// What a released touch claim turned into — [`TouchFling::release`].
#[derive(Debug)]
pub enum TouchRelease {
    /// The release earned a fling, which now owns the offset.
    Fling(Box<TouchFling>),
    /// The release earned none and ended its gesture.
    Ended {
        /// The end clamped an offset held past an edge back into the
        /// extents — a frame must show it.
        moved: bool,
    },
}

/// The outcome of one [`TouchFling::tick`].
#[derive(Debug)]
pub struct TouchFlingTick {
    /// The fling wrote a new offset this tick — a frame must present it,
    /// including the tick the fling settles on.
    pub(crate) changed: bool,
    /// The fling still animates — keep it alive and keep pumping frames.
    pub(crate) running: bool,
}

/// One axis of an [`OverScroller`]-model fling: the spline deceleration
/// from AOSP `frameworks/base/core/java/android/widget/OverScroller.java`
/// (`SplineScroller`, SPLINE state) — a cubic-Bézier distance curve over a
/// fixed sample table, with `OverScroller`'s own edge/overscroll states out
/// of scope: the run ends on the clamped edge.
#[derive(Debug)]
struct SplineFling {
    started_at: Instant,
    /// The run's length — `mDuration`: `getSplineFlingDuration`'s value,
    /// shrunk by `adjustDuration` when the unclamped spline overshoots the
    /// content edge.
    duration: Duration,
    /// `mSplineDuration` — the unadjusted duration the position curve's
    /// time fraction divides by.
    spline_duration: Duration,
    /// Offset at release — `mStart`.
    start: f64,
    /// The unclamped signed spline distance the position curve travels —
    /// `mSplineDistance`. The run ends at the clamped `final`, which the
    /// shortened `duration` places exactly on the edge.
    spline_distance: f64,
    /// The clamped offset the run ends at — `mFinal` after the edge clamp.
    final_offset: f64,
}

impl SplineFling {
    /// `SplineScroller.fling`: the spline's duration and distance derive
    /// from the clamped velocity through the platform's physical
    /// coefficient and friction — `getSplineFlingDuration`,
    /// `getSplineFlingDistance` and the `adjustDuration` the edge clamp
    /// applies. `velocity` is the offset's signed speed in logical units
    /// per second, `extent` its maximum.
    fn new(
        velocity: f64,
        start: f64,
        extent: f64,
        physical_coeff: f64,
        friction: f64,
        at: Instant,
    ) -> Self {
        // AOSP `getSplineDeceleration` — `DECELERATION_RATE = ln(0.78)/ln(0.9)`.
        let deceleration_rate = 0.78_f64.log(0.9);
        let deceleration_minus_one = deceleration_rate - 1.0;
        let spline_deceleration = (INFLEXION * velocity.abs() / (friction * physical_coeff)).ln();
        let spline_duration =
            Duration::from_secs_f64((spline_deceleration / deceleration_minus_one).exp());
        let mut duration = spline_duration;
        // `getSplineFlingDistance` returns the run's magnitude; the sign
        // applies separately (`mSplineDistance = totalDistance * signum`).
        let magnitude = friction
            * physical_coeff
            * (deceleration_rate / deceleration_minus_one * spline_deceleration).exp();
        let spline_distance = magnitude.copysign(velocity);
        let unclamped_final = start + spline_distance;
        let final_offset = unclamped_final.clamp(0.0, extent);
        if !(0.0..=extent).contains(&unclamped_final) {
            // `adjustDuration`: a clamped run ends where the edge is, at
            // the spline-time fraction of the clamped distance fraction —
            // not by crawling through the unclamped spline's full span.
            let edge_fraction = ((final_offset - start) / spline_distance).abs();
            duration = spline_duration.mul_f64(spline_time(edge_fraction));
        }
        Self {
            started_at: at,
            duration,
            spline_duration,
            start,
            spline_distance,
            final_offset,
        }
    }

    /// The axis's offset at `now` and whether the run still animates —
    /// `SplineOverScroller.update`'s SPLINE state: the sampled
    /// `SPLINE_POSITION` fraction of the unclamped spline distance, until
    /// `mDuration` (adjusted for the edge) reports the run finished at the
    /// clamped `mFinal`.
    fn position(&self, now: Instant) -> (f64, bool) {
        let elapsed = now.saturating_duration_since(self.started_at);
        if elapsed >= self.duration {
            return (self.final_offset, false);
        }
        let t = elapsed.as_secs_f64() / self.spline_duration.as_secs_f64();
        (
            spline_position(t).mul_add(self.spline_distance, self.start),
            true,
        )
    }
}

/// `SplineScroller`'s `INFLEXION` — the tension curves cross at
/// (INFLEXION, 1).
const INFLEXION: f64 = 0.35;
const START_TENSION: f64 = 0.5;
const END_TENSION: f64 = 1.0;
const P1: f64 = START_TENSION * INFLEXION;
const P2: f64 = 1.0 - END_TENSION * (1.0 - INFLEXION);
/// `SPLINE_POSITION`/`SPLINE_TIME` sample count.
const SPLINE_SAMPLES: usize = 100;
/// Remaining fling distance under which a started run is dropped.
const SCROLL_END_EPSILON: f64 = 0.000_1;

/// `SplineScroller`'s `SPLINE_POSITION` and `SPLINE_TIME` tables — the
/// distance fraction at each time fraction, and its inverse — built by the
/// same bisection the static initializer in `OverScroller.java` runs.
const SPLINE_POSITION: [f64; SPLINE_SAMPLES + 1] = build_spline_tables().0;
const SPLINE_TIME: [f64; SPLINE_SAMPLES + 1] = build_spline_tables().1;

const fn build_spline_tables() -> ([f64; SPLINE_SAMPLES + 1], [f64; SPLINE_SAMPLES + 1]) {
    let mut position = [0.0; SPLINE_SAMPLES + 1];
    let mut time = [0.0; SPLINE_SAMPLES + 1];
    let mut x_min = 0.0;
    let mut y_min = 0.0;
    let mut i = 0;
    while i < SPLINE_SAMPLES {
        let alpha =
            crate::num_cast::usize_as_f64(i) / crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
        let mut x_max = 1.0;
        let mut x;
        loop {
            x = x_min + (x_max - x_min) / 2.0;
            let coef = 3.0 * x * (1.0 - x);
            let tx = coef * ((1.0 - x) * P1 + x * P2) + x * x * x;
            if (if tx < alpha { alpha - tx } else { tx - alpha }) < 1e-5 {
                break;
            }
            if tx > alpha {
                x_max = x;
            } else {
                x_min = x;
            }
        }
        let coef = 3.0 * x * (1.0 - x);
        position[i] = coef * ((1.0 - x) * START_TENSION + x) + x * x * x;
        let mut y_max = 1.0;
        let mut y;
        loop {
            y = y_min + (y_max - y_min) / 2.0;
            let coef = 3.0 * y * (1.0 - y);
            let dy = coef * ((1.0 - y) * START_TENSION + y) + y * y * y;
            if (if dy < alpha { alpha - dy } else { dy - alpha }) < 1e-5 {
                break;
            }
            if dy > alpha {
                y_max = y;
            } else {
                y_min = y;
            }
        }
        let coef = 3.0 * y * (1.0 - y);
        time[i] = coef * ((1.0 - y) * P1 + y * P2) + y * y * y;
        i += 1;
    }
    position[SPLINE_SAMPLES] = 1.0;
    time[SPLINE_SAMPLES] = 1.0;
    (position, time)
}

/// `SplineScroller.update`'s `distanceCoef`: the `SPLINE_POSITION` value at
/// `t` with the same linear interpolation between adjacent samples.
fn spline_position(t: f64) -> f64 {
    let scaled = t * crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
    let index = crate::num_cast::f64_as_usize(scaled);
    if index >= SPLINE_SAMPLES {
        return 1.0;
    }
    let low = SPLINE_POSITION[index];
    let high = SPLINE_POSITION[index + 1];
    (scaled - crate::num_cast::usize_as_f64(index)).mul_add(high - low, low)
}

/// `SplineScroller.adjustDuration`'s `timeCoef`: the `SPLINE_TIME` value at
/// the clamped distance fraction `x`, interpolated the same way.
fn spline_time(x: f64) -> f64 {
    let scaled = x * crate::num_cast::usize_as_f64(SPLINE_SAMPLES);
    let index = crate::num_cast::f64_as_usize(scaled);
    if index >= SPLINE_SAMPLES {
        return 1.0;
    }
    let low = SPLINE_TIME[index];
    let high = SPLINE_TIME[index + 1];
    (scaled - crate::num_cast::usize_as_f64(index)).mul_add(high - low, low)
}
