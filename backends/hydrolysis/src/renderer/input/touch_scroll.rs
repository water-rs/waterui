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
//! the platform's deceleration model and clamps at the content edges; a new
//! touch down stops the fling where it is.
//!
//! The gesture's parameters are the host's [`TouchScrollConfig`], pushed
//! into the renderer by value from `PlatformWindow::touch_scroll_config` —
//! no platform values are invented here.
// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform::TouchScrollConfig;
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
}

/// A scroll view's claimed drag, scrolled one to one by later moves.
#[derive(Debug)]
pub struct TouchScrollDrag {
    /// The claimed scroll view's offset handle.
    pub(crate) handle: crate::scroll::ScrollHandle,
    /// The point the last applied delta ended at, in window hit-test space.
    pub(crate) last: kurbo::Point,
    /// The motion samples the release's fling is fit over.
    pub(crate) tracker: VelocityTracker,
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

/// A fling running after the touch released: per-axis spline deceleration
/// driving the claimed scroll view's handle until it settles — inside the
/// content bounds — or a new touch down drops it.
#[derive(Debug)]
pub struct TouchFling {
    handle: crate::scroll::ScrollHandle,
    /// The claim the fling holds on the offset — the ownership record: its
    /// mint already claimed the offset from any live run or wheel glide, it
    /// keeps writing while it is still the newest claim, and its first
    /// refused write — a request, a jump, user input — ends it rather than
    /// overwriting its successor. Neither an anchor's `rebase` nor a
    /// `rebind` for rows measured mid-fling touches it.
    claim: crate::scroll::FlingClaim,
    x: Option<SplineFling>,
    y: Option<SplineFling>,
}

impl TouchFling {
    /// Starts a fling on `handle` from the fitted pointer velocity, in
    /// logical units per second: the offset's velocity is the finger's
    /// negated, clamped to the host's maximum, and an axis under the
    /// minimum — or already at the edge it heads toward — does not fling.
    /// `None` when no axis earns a fling.
    pub(crate) fn start(
        handle: crate::scroll::ScrollHandle,
        finger_velocity: kurbo::Vec2,
        config: &TouchScrollConfig,
        at: Instant,
    ) -> Option<Self> {
        let metrics = handle.metrics();
        let max = f64::from(config.max_fling_velocity);
        let min = f64::from(config.min_fling_velocity);
        let axis_fling = |velocity: f64, start: f64, extent: f64| {
            // The content offset moves opposite the finger: a finger flung
            // up pushes the offset down into the content.
            let offset_velocity = (-velocity).clamp(-max, max);
            (offset_velocity.abs() >= min)
                .then(|| {
                    SplineFling::new(
                        offset_velocity,
                        start,
                        extent,
                        config.fling.physical_coeff,
                        config.fling.friction,
                        at,
                    )
                })
                // A run whose clamped destination is already the offset —
                // released at the edge it headed toward — is no fling.
                .filter(|fling| (fling.final_offset - fling.start).abs() > SCROLL_END_EPSILON)
        };
        let x = matches!(handle.axis(), Axis::Horizontal | Axis::All)
            .then(|| axis_fling(finger_velocity.x, metrics.offset_x, metrics.max_x))
            .flatten();
        let y = matches!(handle.axis(), Axis::Vertical | Axis::All)
            .then(|| axis_fling(finger_velocity.y, metrics.offset_y, metrics.max_y))
            .flatten();
        // The mint itself claims the offset — ending a live run or wheel
        // glide — so it must wait until a fling is known to exist.
        (x.is_some() || y.is_some()).then(|| Self {
            claim: handle.begin_fling(),
            handle,
            x,
            y,
        })
    }

    /// Advances the fling to `now` and applies its offset through the
    /// handle.
    pub(crate) fn tick(&self, now: Instant) -> TouchFlingTick {
        let metrics = self.handle.metrics();
        let (offset_x, active_x) = self
            .x
            .as_ref()
            .map_or((metrics.offset_x, false), |fling| fling.position(now));
        let (offset_y, active_y) = self
            .y
            .as_ref()
            .map_or((metrics.offset_y, false), |fling| fling.position(now));
        // A programmatic request — jump or animated — or any user input
        // claims the offset past this fling's claim: the refused write
        // ends the fling instead of writing over whatever replaced it.
        let changed = self
            .handle
            .apply_fling_offset(&self.claim, offset_x, offset_y);
        TouchFlingTick {
            changed,
            // A refused write — a newer claim on the offset — ends the
            // fling on the spot.
            running: changed && (active_x || active_y),
        }
    }
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
/// fixed sample table, with the edge/overscroll states out of scope since
/// offsets clamp at the content bounds.
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
