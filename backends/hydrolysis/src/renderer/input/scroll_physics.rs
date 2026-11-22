//! The `UIScrollView` physics family's math: the exponential deceleration,
//! the rubber band a drag past an edge meets, the return a release past an
//! edge makes and the bounce a fling into one makes — each in closed form
//! over the time since its run began, so a run is a pure function of the
//! clock.
//!
//! The constants the formulas run on are the family's own fields
//! ([`ExponentialDecay`], [`RubberBand`], [`ReleaseReturn`], [`EdgeBounce`]
//! and [`SpringBack`] in `crate::platform`), each with its source cited
//! there.

use crate::platform::{EdgeBounce, ExponentialDecay, ReleaseReturn, RubberBand, SpringBack};
use crate::time::Instant;

/// The displacement under which a run counts as at rest, in logical units:
/// a third of a physical pixel at 3x — the highest scale of current
/// iPhones — so the final snap onto the resting position is invisible. The
/// same 0.1 the shared scroll state settles its wheel glide within.
pub(super) const REST_DISTANCE: f64 = 0.1;

/// The speed under which a spring counts as at rest, in logical units per
/// second: it moves less than [`REST_DISTANCE`] in one frame at the 120 Hz
/// the renderer targets.
pub(super) const REST_SPEED: f64 = REST_DISTANCE * 120.0;

/// Milliseconds per second — the decay rate is a per-millisecond factor
/// and velocities are per second.
const MS_PER_SECOND: f64 = 1000.0;

impl ExponentialDecay {
    /// The velocity `seconds` after a release at `velocity`:
    /// `v0 · rate^t`, `t` in milliseconds.
    pub(crate) fn velocity(self, velocity: f64, seconds: f64) -> f64 {
        velocity * self.rate_per_ms().powf(seconds * MS_PER_SECOND)
    }

    /// The distance a release at `velocity` travels in total — WWDC 2018
    /// "Designing Fluid Interfaces"' projection, `v0/1000 · rate/(1 − rate)`.
    pub(crate) fn projection(self, velocity: f64) -> f64 {
        velocity / MS_PER_SECOND * self.rate_per_ms() / (1.0 - self.rate_per_ms())
    }

    /// The distance travelled `seconds` after a release at `velocity`: the
    /// per-millisecond recurrence `v[n+1] = v[n] · rate`, `x[n+1] = x[n] +
    /// v[n+1]/1000` summed in closed form, `projection · (1 − rate^t)` —
    /// whose limit is [`Self::projection`] exactly.
    pub(crate) fn distance(self, velocity: f64, seconds: f64) -> f64 {
        self.projection(velocity) * (1.0 - self.rate_per_ms().powf(seconds * MS_PER_SECOND))
    }

    /// When a release at `velocity` has travelled `distance` — same sign —
    /// or `None` when its projection falls short of it.
    pub(crate) fn time_to_distance(self, velocity: f64, distance: f64) -> Option<f64> {
        let remaining = 1.0 - distance / self.projection(velocity);
        (remaining > 0.0).then(|| remaining.ln() / (self.rate_per_ms().ln() * MS_PER_SECOND))
    }

    /// When a release at `velocity` comes to rest: the moment the distance
    /// still ahead of it, `|projection| · rate^t`, falls under
    /// [`REST_DISTANCE`]. Zero for a release that projects no further.
    pub(crate) fn rest_time(self, velocity: f64) -> f64 {
        let projection = self.projection(velocity).abs();
        if projection <= REST_DISTANCE {
            return 0.0;
        }
        (REST_DISTANCE / projection).ln() / (self.rate_per_ms().ln() * MS_PER_SECOND)
    }
}

impl RubberBand {
    /// How far a drag `overscroll` past an edge shows the content moved:
    /// `(1 − 1 / (x·c/d + 1)) · d` for the viewport's `dimension` `d` —
    /// slope `c` at the edge, approaching but never reaching `d`.
    pub(crate) fn band(self, overscroll: f64, dimension: f64) -> f64 {
        (1.0 - 1.0 / (overscroll * self.coefficient() / dimension + 1.0)) * dimension
    }

    /// The drag past an edge that shows `banded` — the inverse of
    /// [`Self::band`], `y / (c · (1 − y/d))`, defined for `banded < d`.
    pub(crate) fn unband(self, banded: f64, dimension: f64) -> f64 {
        banded / (self.coefficient() * (1.0 - banded / dimension))
    }

    /// The offset a drag moves `offset` to by `delta` on an axis whose
    /// extents are `0..=max` and whose viewport is `dimension` long: one to
    /// one inside the extents, banded past them. The live offset is
    /// unbanded first, so the drag stays continuous across an edge and
    /// carries over from wherever a rebind or a caught bounce left it.
    ///
    /// An axis with nothing to scroll — `max` zero, or no viewport — does
    /// not bounce: `UIScrollView`'s `alwaysBounceVertical` and
    /// `alwaysBounceHorizontal` are `false` by default (Apple Developer
    /// Documentation, `UIScrollView.alwaysBounceVertical`), so a drag along
    /// such an axis leaves it at its edge.
    pub(crate) fn drag(self, offset: f64, delta: f64, max: f64, dimension: f64) -> f64 {
        if max <= 0.0 || dimension <= 0.0 {
            return 0.0;
        }
        let raw = if offset < 0.0 {
            -self.unband(-offset, dimension)
        } else if offset > max {
            max + self.unband(offset - max, dimension)
        } else {
            offset
        } + delta;
        if raw < 0.0 {
            -self.band(-raw, dimension)
        } else if raw > max {
            max + self.band(raw - max, dimension)
        } else {
            raw
        }
    }
}

/// A motion that carries an offset back onto an edge: its displacement
/// from the edge and its velocity `t` seconds after it starts at
/// displacement `x0` with velocity `v0`.
pub(super) trait EdgeReturn: Copy + core::fmt::Debug {
    fn sample(self, x0: f64, v0: f64, t: f64) -> (f64, f64);
}

/// Where a release moving toward the content hands over to a fling inside
/// it: `seconds` after the release, on the edge, at `velocity` — in
/// logical units per second.
#[derive(Debug, Clone, Copy)]
pub(super) struct Handover {
    pub(super) seconds: f64,
    pub(super) velocity: f64,
}

impl ReleaseReturn {
    /// The handover of a release `x0` past the edge at `v0`: at `t_c = −x0
    /// / v0`, at `v0 · e^(−ω_h·t_c)` — `None` unless the release moves
    /// toward the content.
    pub(super) fn handover(self, x0: f64, v0: f64) -> Option<Handover> {
        (x0 * v0 < 0.0).then(|| {
            let seconds = -x0 / v0;
            let omega = core::f64::consts::TAU / self.handover_response();
            Handover {
                seconds,
                velocity: v0 * (-omega * seconds).exp(),
            }
        })
    }
}

impl EdgeReturn for ReleaseReturn {
    /// `(x0 + v0·t) · e^(−ω·t)` and its derivative, `(v0 − ω·(x0 + v0·t))
    /// · e^(−ω·t)` — the closed form [`ReleaseReturn`] documents.
    fn sample(self, x0: f64, v0: f64, t: f64) -> (f64, f64) {
        let omega = core::f64::consts::TAU / self.response();
        let decay = (-omega * t).exp();
        let x = v0.mul_add(t, x0);
        (x * decay, omega.mul_add(-x, v0) * decay)
    }
}

impl EdgeReturn for SpringBack {
    /// The closed forms [`SpringBack`] documents, with `ω0 = 2π /
    /// response`.
    fn sample(self, x0: f64, v0: f64, t: f64) -> (f64, f64) {
        let omega = core::f64::consts::TAU / self.response();
        let zeta = self.damping_ratio();
        if (zeta - 1.0).abs() < f64::EPSILON {
            let b = omega.mul_add(x0, v0);
            let decay = (-omega * t).exp();
            let x = b.mul_add(t, x0);
            (x * decay, omega.mul_add(-x, b) * decay)
        } else if zeta > 1.0 {
            let root = zeta.mul_add(zeta, -1.0).sqrt();
            let r1 = -omega * (zeta - root);
            let r2 = -omega * (zeta + root);
            let c1 = r2.mul_add(-x0, v0) / (r1 - r2);
            let c2 = x0 - c1;
            let (e1, e2) = ((r1 * t).exp(), (r2 * t).exp());
            (c1.mul_add(e1, c2 * e2), (c1 * r1).mul_add(e1, c2 * r2 * e2))
        } else {
            let damping = zeta * omega;
            let omega_d = omega * zeta.mul_add(-zeta, 1.0).sqrt();
            let b = damping.mul_add(x0, v0) / omega_d;
            let decay = (-damping * t).exp();
            let (sin, cos) = (omega_d * t).sin_cos();
            (
                decay * x0.mul_add(cos, b * sin),
                decay
                    * damping
                        .mul_add(-x0, b * omega_d)
                        .mul_add(cos, -damping.mul_add(b, x0 * omega_d) * sin),
            )
        }
    }
}

/// An axis's offset returning onto an edge: from a release past it, or
/// from a fling that crossed it.
#[derive(Debug, Clone, Copy)]
pub(super) struct SpringRun<R> {
    /// The edge the run returns to.
    target: f64,
    /// The displacement from the edge it starts at.
    x0: f64,
    /// The offset's velocity it starts with, in logical units per second.
    v0: f64,
    motion: R,
}

impl<R: EdgeReturn> SpringRun<R> {
    pub(super) const fn new(target: f64, x0: f64, v0: f64, motion: R) -> Self {
        Self {
            target,
            x0,
            v0,
            motion,
        }
    }

    /// The offset `seconds` into the run and whether it still moves. The
    /// run ends exactly on the edge once it is at rest there — or the
    /// moment it would cross it: a return never carries the offset past
    /// the edge into the content.
    pub(super) fn at(&self, seconds: f64) -> (f64, bool) {
        let (x, v) = self.motion.sample(self.x0, self.v0, seconds);
        // The side of the edge the run lives on: where it starts, or — for
        // a bounce starting on the edge — where its velocity carries it.
        let side = if self.x0.abs() > 0.0 {
            self.x0
        } else {
            self.v0
        };
        let crossed = x * side < 0.0;
        if crossed || (x.abs() < REST_DISTANCE && v.abs() < REST_SPEED) {
            return (self.target, false);
        }
        (self.target + x, true)
    }
}

/// One axis of a fling under [`ExponentialDecay`]: decay from the release
/// until it rests — or until it reaches an edge, where it either stops
/// (clamped edges) or springs out and back ([`RubberBand`] edges).
#[derive(Debug)]
pub(super) struct DecayFling {
    started_at: Instant,
    start: f64,
    velocity: f64,
    decay: ExponentialDecay,
    /// How long the decay runs, in seconds: to rest, or to the edge.
    duration: f64,
    end: DecayEnd,
}

/// What follows a [`DecayFling`]'s decay.
#[derive(Debug)]
enum DecayEnd {
    /// It came to rest inside the extents.
    Rest,
    /// It reached `edge` and stops there.
    Edge(f64),
    /// It reached an edge and bounces: the spring starts on the edge with
    /// the bounce's share of the decay's velocity there.
    Bounce(SpringRun<SpringBack>),
}

impl DecayFling {
    /// A fling of the offset at `start`, inside `0..=extent`, released at
    /// `velocity`; `bounce` is the spring an edge it reaches bounces it
    /// on, `None` for an edge that stops it. `None` when the fling moves
    /// nothing: it rests at once, or stops on the edge it starts at.
    pub(super) fn new(
        decay: ExponentialDecay,
        velocity: f64,
        start: f64,
        extent: f64,
        bounce: Option<EdgeBounce>,
        at: Instant,
    ) -> Option<Self> {
        let rest = decay.rest_time(velocity);
        let edge = if velocity > 0.0 { extent } else { 0.0 };
        let crossing = decay
            .time_to_distance(velocity, edge - start)
            .filter(|&crossing| crossing <= rest);
        let (duration, end) = match (crossing, bounce) {
            (None, _) => (rest, DecayEnd::Rest),
            (Some(crossing), None) => (crossing, DecayEnd::Edge(edge)),
            (Some(crossing), Some(bounce)) => (
                crossing,
                DecayEnd::Bounce(SpringRun::new(
                    edge,
                    0.0,
                    bounce.entry_velocity_scale() * decay.velocity(velocity, crossing),
                    bounce.spring(),
                )),
            ),
        };
        let moves = match end {
            DecayEnd::Rest => duration > 0.0,
            DecayEnd::Edge(edge) => (edge - start).abs() > REST_DISTANCE,
            DecayEnd::Bounce(_) => true,
        };
        moves.then_some(Self {
            started_at: at,
            start,
            velocity,
            decay,
            duration,
            end,
        })
    }

    /// The offset at `now` and whether the fling still moves.
    pub(super) fn position(&self, now: Instant) -> (f64, bool) {
        let elapsed = now.saturating_duration_since(self.started_at).as_secs_f64();
        if elapsed < self.duration {
            return (
                self.start + self.decay.distance(self.velocity, elapsed),
                true,
            );
        }
        match &self.end {
            DecayEnd::Rest => (
                self.start + self.decay.distance(self.velocity, self.duration),
                false,
            ),
            DecayEnd::Edge(edge) => (*edge, false),
            DecayEnd::Bounce(spring) => spring.at(elapsed - self.duration),
        }
    }

    /// When a fling that stops on an edge reaches it, and its velocity
    /// there; `None` for one that rests inside or bounces.
    pub(super) fn edge_arrival(&self) -> Option<(Instant, f64)> {
        matches!(self.end, DecayEnd::Edge(_)).then(|| {
            (
                self.started_at + core::time::Duration::from_secs_f64(self.duration),
                self.decay.velocity(self.velocity, self.duration),
            )
        })
    }
}

/// One axis released past an edge: the return onto it.
#[derive(Debug)]
pub(super) struct SpringFling {
    started_at: Instant,
    run: SpringRun<ReleaseReturn>,
}

impl SpringFling {
    /// A release of the offset `overscroll` past `edge` with the offset
    /// moving at `velocity`.
    pub(super) const fn new(
        edge: f64,
        overscroll: f64,
        velocity: f64,
        release: ReleaseReturn,
        at: Instant,
    ) -> Self {
        Self {
            started_at: at,
            run: SpringRun::new(edge, overscroll, velocity, release),
        }
    }

    /// The offset at `now` and whether the return still moves.
    pub(super) fn position(&self, now: Instant) -> (f64, bool) {
        self.run
            .at(now.saturating_duration_since(self.started_at).as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    const DECAY: ExponentialDecay = ExponentialDecay::NORMAL;

    /// Critically damped and overdamped springs over a spread of
    /// responses: the properties below hold for any of them, not just the
    /// family's current fit.
    fn non_oscillating_springs() -> Vec<SpringBack> {
        let mut springs = Vec::new();
        for response in [0.2, 0.4, 0.55, 1.0] {
            for damping_ratio in [1.0, 1.2, 2.0] {
                springs.push(SpringBack::new(response, damping_ratio));
            }
        }
        springs
    }

    fn band() -> RubberBand {
        RubberBand::UIKIT
    }

    /// Edge bounces on each of [`non_oscillating_springs`], entering at the
    /// crossing velocity itself.
    fn bounces() -> Vec<EdgeBounce> {
        non_oscillating_springs()
            .into_iter()
            .map(|spring| EdgeBounce::new(spring, 1.0))
            .collect()
    }

    #[test]
    fn decay_velocity_and_distance_follow_the_closed_form() {
        let v0 = 2_000.0;
        for ms in [0_u32, 1, 16, 100, 500, 2_000] {
            let t = f64::from(ms) / 1000.0;
            let expected_v = v0 * 0.998_f64.powi(i32::try_from(ms).unwrap());
            assert!((DECAY.velocity(v0, t) - expected_v).abs() < 1e-9 * v0);
            // The per-millisecond recurrence the closed form sums.
            let (mut x, mut v) = (0.0, v0);
            for _ in 0..ms {
                v *= 0.998;
                x += v / 1000.0;
            }
            assert!(
                (DECAY.distance(v0, t) - x).abs() < 1e-6,
                "{ms} ms: {} vs {x}",
                DECAY.distance(v0, t)
            );
        }
    }

    #[test]
    fn decay_projects_to_the_wwdc_projection() {
        for v0 in [-3_000.0, -120.0, 500.0, 6_000.0] {
            let projection = v0 / 1000.0 * 0.998 / (1.0 - 0.998);
            assert!((DECAY.projection(v0) - projection).abs() < 1e-9);
            assert!((DECAY.distance(v0, 60.0) - projection).abs() < 1e-6);
            // At rest the run is within the rest distance of the projection.
            let rest = DECAY.rest_time(v0);
            assert!((DECAY.distance(v0, rest) - projection).abs() <= REST_DISTANCE + 1e-9);
        }
    }

    #[test]
    fn decay_reaches_a_distance_at_the_solved_time() {
        let v0 = 1_500.0;
        let t = DECAY
            .time_to_distance(v0, 400.0)
            .expect("400pt is within the projection");
        assert!((DECAY.distance(v0, t) - 400.0).abs() < 1e-9);
        assert!(
            DECAY
                .time_to_distance(v0, DECAY.projection(v0) + 1.0)
                .is_none()
        );
    }

    #[test]
    fn rubber_band_is_monotonic_bounded_and_starts_at_slope_c() {
        let dimension = 600.0;
        let mut previous = 0.0;
        for step in 1..=2_000 {
            let x = f64::from(step) * 5.0;
            let banded = band().band(x, dimension);
            assert!(banded > previous, "monotonic at {x}");
            assert!(banded < dimension, "bounded by the dimension at {x}");
            assert!(banded < x, "resists at {x}");
            previous = banded;
        }
        let slope = band().band(1e-6, dimension) / 1e-6;
        assert!((slope - 0.55).abs() < 1e-6, "slope at the edge {slope}");
        // The inverse recovers the drag.
        for x in [0.5, 40.0, 900.0] {
            let banded = band().band(x, dimension);
            assert!((band().unband(banded, dimension) - x).abs() < 1e-9);
        }
    }

    #[test]
    fn a_drag_is_one_to_one_inside_and_banded_past_an_edge() {
        let (max, dimension) = (1_000.0, 600.0);
        assert!((band().drag(300.0, 50.0, max, dimension) - 350.0).abs() < 1e-12);
        // Past the top edge, by 60pt of finger travel.
        let past_top = band().drag(20.0, -80.0, max, dimension);
        assert!((past_top + band().band(60.0, dimension)).abs() < 1e-9);
        // Dragging back retraces the band and lands in the content again.
        let back = band().drag(past_top, 80.0, max, dimension);
        assert!((back - 20.0).abs() < 1e-9);
        // Past the bottom edge.
        let past_bottom = band().drag(990.0, 110.0, max, dimension);
        assert!((past_bottom - (max + band().band(100.0, dimension))).abs() < 1e-9);
        // An axis with nothing to scroll never bounces.
        assert!(band().drag(0.0, -80.0, 0.0, dimension).abs() < f64::EPSILON);
    }

    /// Samples a run every 1 ms until it ends and returns the offsets.
    fn trace(run: impl Fn(f64) -> (f64, bool)) -> (Vec<f64>, f64) {
        let mut offsets = Vec::new();
        for ms in 0..20_000_u32 {
            let (offset, active) = run(f64::from(ms) / 1000.0);
            offsets.push(offset);
            if !active {
                return (offsets, offset);
            }
        }
        panic!("the run never came to rest");
    }

    #[test]
    fn a_release_past_an_edge_returns_monotonically_and_settles_on_it() {
        for response in [0.3, ReleaseReturn::UIKIT.response(), 1.0] {
            let release = {
                let mut literal = ReleaseReturn::UIKIT;
                literal.set_response(response);
                literal
            };
            // Released 80pt past the top edge, still, moving further out,
            // and moving back in.
            for v0 in [0.0, -400.0, 300.0] {
                let run = SpringRun::new(0.0, -80.0, v0, release);
                let (offsets, end) = trace(|t| run.at(t));
                assert!(
                    end.abs() < f64::EPSILON,
                    "{release:?} {v0}: settles exactly on the edge"
                );
                assert!(
                    offsets.iter().all(|&offset| offset <= 0.0),
                    "{release:?} {v0}: never past the edge into the content"
                );
                // Once heading back, it never turns away from the edge.
                let turn = offsets
                    .windows(2)
                    .position(|pair| pair[1] > pair[0])
                    .unwrap_or(0);
                assert!(
                    offsets[turn..].windows(2).all(|pair| pair[1] >= pair[0]),
                    "{release:?} {v0}: monotonic return"
                );
            }
            // A still release leaves at once: (x0 + v·t)·e^(−ω·t) starts at
            // velocity −ω·x0, not 0.
            let (_, v) = release.sample(-80.0, 0.0, 0.0);
            assert!((core::f64::consts::TAU / response).mul_add(-80.0, v).abs() < 1e-9);
            // Past the bottom edge of a 500pt extent.
            let run = SpringRun::new(500.0, 40.0, 0.0, release);
            let (offsets, end) = trace(|t| run.at(t));
            assert!((end - 500.0).abs() < f64::EPSILON);
            assert!(offsets.windows(2).all(|pair| pair[1] <= pair[0]));
            assert!(offsets.iter().all(|&offset| offset >= 500.0));
        }
    }

    /// The family's constants reproduce the `UIScrollView` traces they were
    /// fitted to (iPad Pro 13-inch (M4), iPadOS 26.5): the displacement
    /// past the top edge, in points, at milliseconds after the start.
    #[test]
    fn the_uikit_return_and_bounce_follow_the_measured_traces() {
        // A release 329pt past the edge with the offset at −123 pt/s.
        let release = SpringRun::new(0.0, -329.0, -123.0, ReleaseReturn::UIKIT);
        for (ms, measured) in [
            (25, -258.5),
            (50, -202.5),
            (100, -124.5),
            (200, -47.0),
            (400, -7.0),
        ] {
            let (offset, _) = release.at(f64::from(ms) / 1000.0);
            assert!(
                (offset - measured).abs() < 1.0,
                "release at {ms} ms: {offset} vs {measured}"
            );
        }
        // A fling crossing the edge at −2470 pt/s.
        let bounce = EdgeBounce::UIKIT;
        let run = SpringRun::new(
            0.0,
            0.0,
            -2470.0 * bounce.entry_velocity_scale(),
            bounce.spring(),
        );
        for (ms, measured) in [
            (25, -45.1),
            (50, -68.7),
            (90, -79.5),
            (200, -52.9),
            (400, -11.8),
        ] {
            let (offset, _) = run.at(f64::from(ms) / 1000.0);
            assert!(
                (offset - measured).abs() < 1.0,
                "bounce at {ms} ms: {offset} vs {measured}"
            );
        }
    }

    /// A release past the top edge moving toward the content, composed as
    /// the touch fling composes it: the return until the handover, then the
    /// decay from the edge. Its offset at `seconds` and whether it moves.
    fn toward_content(x0: f64, v0: f64) -> impl Fn(f64) -> (f64, bool) {
        let release = ReleaseReturn::UIKIT;
        let run = SpringRun::new(0.0, x0, v0, release);
        let at = Instant::now();
        let then = release.handover(x0, v0).and_then(|handover| {
            DecayFling::new(
                DECAY,
                handover.velocity,
                0.0,
                100_000.0,
                Some(EdgeBounce::UIKIT),
                at + Duration::from_secs_f64(handover.seconds),
            )
            .map(|fling| (handover.seconds, fling))
        });
        move |seconds| match &then {
            Some((from, fling)) if seconds >= *from => {
                fling.position(at + Duration::from_secs_f64(seconds))
            }
            _ => run.at(seconds),
        }
    }

    /// Releases past the top edge while the finger moves back toward the
    /// content (the same device, `scrollViewWillEndDragging` velocity,
    /// offsets in points after the release): a fast one carries into the
    /// content and decelerates there, a slow one stops on the edge.
    #[test]
    fn a_uikit_release_toward_the_content_follows_the_measured_traces() {
        for (x0, v0, points, rest) in [
            (
                -44.5,
                2139.2,
                &[(200, 248.5), (400, 429.0), (1000, 685.5)][..],
                791.0,
            ),
            (
                -183.5,
                1530.0,
                &[(200, 16.5), (400, 57.0), (1000, 115.0)][..],
                135.0,
            ),
            (
                -183.5,
                336.0,
                &[(100, -56.0), (200, -17.0), (400, -1.5)][..],
                0.0,
            ),
        ] {
            let run = toward_content(x0, v0);
            for &(ms, measured) in points {
                let (offset, _) = run(f64::from(ms) / 1000.0);
                assert!(
                    (offset - measured).abs() < 10.0,
                    "{x0} at {v0}: {offset} vs {measured} at {ms} ms"
                );
            }
            let (_, end) = trace(&run);
            assert!(
                (end - rest).abs() < 10.0,
                "{x0} at {v0}: rests at {end} vs {rest}"
            );
            if rest == 0.0 {
                assert!(end.abs() < f64::EPSILON, "{x0} at {v0}: stops on the edge");
            }
        }
        // A release moving deeper hands over to nothing.
        assert!(ReleaseReturn::UIKIT.handover(-80.0, -300.0).is_none());
        assert!(ReleaseReturn::UIKIT.handover(-80.0, 0.0).is_none());
        // Past the bottom edge the handover heads back up into the content.
        let handover = ReleaseReturn::UIKIT
            .handover(60.0, -900.0)
            .expect("moving toward the content");
        assert!(handover.velocity < 0.0 && handover.velocity > -900.0);
        assert!((handover.seconds - 60.0 / 900.0).abs() < 1e-12);
    }

    #[test]
    fn a_fling_into_an_edge_bounces_out_and_back_onto_it() {
        let at = Instant::now();
        for bounce in bounces() {
            // 100pt from the bottom of a 400pt extent, flung down hard.
            let fling = DecayFling::new(DECAY, 3_000.0, 300.0, 400.0, Some(bounce), at)
                .expect("the fling moves");
            let (offsets, end) = trace(|t| fling.position(at + Duration::from_secs_f64(t)));
            assert!(
                (end - 400.0).abs() < f64::EPSILON,
                "{bounce:?}: back on the edge"
            );
            let peak = offsets.iter().copied().fold(f64::MIN, f64::max);
            assert!(
                peak > 400.0 + REST_DISTANCE,
                "{bounce:?}: it bounced past the edge"
            );
            assert!(
                offsets
                    .windows(2)
                    .all(|pair| pair[1] >= pair[0] || pair[1] >= 400.0)
            );
            assert!(offsets.iter().all(|&offset| offset >= 300.0 - 1e-9));
        }
    }

    #[test]
    fn a_fling_short_of_the_edge_rests_at_its_projection() {
        let at = Instant::now();
        let fling = DecayFling::new(DECAY, 800.0, 100.0, 5_000.0, Some(EdgeBounce::UIKIT), at)
            .expect("the fling moves");
        let (_, end) = trace(|t| fling.position(at + Duration::from_secs_f64(t)));
        assert!((end - (100.0 + DECAY.projection(800.0))).abs() <= REST_DISTANCE);
    }

    #[test]
    fn a_clamped_fling_stops_on_the_edge() {
        let at = Instant::now();
        let fling =
            DecayFling::new(DECAY, -3_000.0, 50.0, 400.0, None, at).expect("the fling moves");
        let (offsets, end) = trace(|t| fling.position(at + Duration::from_secs_f64(t)));
        assert!(end.abs() < f64::EPSILON);
        assert!(offsets.iter().all(|&offset| offset >= 0.0));
        // Already on the edge it heads toward: no fling.
        assert!(DecayFling::new(DECAY, -3_000.0, 0.0, 400.0, None, at).is_none());
    }
}
