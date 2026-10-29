//! Android 12's stretch overscroll: the edge effect AOSP
//! `frameworks/base/core/java/android/widget/EdgeEffect.java` runs with
//! `TYPE_STRETCH`, one per edge; the pair per axis `ScrollView` drives
//! from a drag, a release and a fling
//! (`frameworks/base/core/java/android/widget/ScrollView.java`); and the
//! stretch HWUI draws from their intensities.
//!
//! The scroll offset never leaves the extents: the effect only stretches
//! the rendered content toward the edge it is pulled from. The stretch is
//! HWUI's stretch shader (`frameworks/base/libs/hwui/effects/StretchEffect.cpp`,
//! `stretchShader`), the edge-weighted warp HWUI draws under its default
//! `StretchEffectBehavior::ShaderHWUI` (`libs/hwui/Properties.h`): the
//! scroll view's inner layer carries filtrate's `OverscrollStretch` in the
//! viewport's frame while [`shown_stretch`] is not empty. It applies at
//! render time, so layout and hit-testing never see it, as on Android,
//! where `RenderNode#stretch` is a draw-time property.
//!
//! HWUI's other rendering, `UniformScale` (`StretchEffect::makeLinearStretch`),
//! is a device-class choice — `Properties::setIsHighEndGfx(false)` — not a
//! case of the stretch itself, so it has no counterpart here.
//!
//! The constants are [`EdgeStretch`]'s, each cited there.

use crate::platform::EdgeStretch;
use crate::time::Instant;

/// `StretchEffect::NON_ZERO_EPSILON`: a stretch intensity at most this far
/// from zero on both axes is no stretch (`StretchEffect::isEmpty`).
const NON_ZERO_EPSILON: f64 = 0.000_04;

/// Java's `Math.signum` as an integer: zero for zero, unlike
/// [`f64::signum`].
fn java_signum(value: f64) -> i8 {
    if value > 0.0 {
        1
    } else if value < 0.0 {
        -1
    } else {
        0
    }
}

/// `EdgeEffect`'s states that the stretch type reaches: `STATE_IDLE`,
/// `STATE_PULL` and `STATE_RECEDE` — `STATE_ABSORB` and `STATE_PULL_DECAY`
/// belong to the glow type only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EdgeState {
    Idle,
    Pull,
    Recede,
}

/// One edge's stretch — `EdgeEffect` with `TYPE_STRETCH`.
#[derive(Debug, Clone)]
pub(super) struct EdgeEffect {
    state: EdgeState,
    /// `mDistance`: the shown pull, as a fraction of the viewport's length
    /// along the axis.
    distance: f64,
    /// `mPullDistance`: the pull a drag accumulates.
    pull_distance: f64,
    /// `mVelocity`: the relaxing spring's speed, in logical units per
    /// second.
    velocity: f64,
    /// `mStartTime`: when the spring last stepped.
    stepped_at: Option<Instant>,
}

impl EdgeEffect {
    const IDLE: Self = Self {
        state: EdgeState::Idle,
        distance: 0.0,
        pull_distance: 0.0,
        velocity: 0.0,
        stepped_at: None,
    };

    /// `isFinished`.
    fn is_finished(&self) -> bool {
        self.state == EdgeState::Idle
    }

    /// `getDistance`.
    pub(super) const fn distance(&self) -> f64 {
        self.distance
    }

    /// `onPull(deltaDistance, displacement)` for the stretch type. The
    /// displacement only steers the glow's horizontal position, so the
    /// stretch ignores it.
    fn on_pull(&mut self, delta: f64) {
        if self.state != EdgeState::Pull {
            // "Catch" the stretch currently showing.
            self.pull_distance = self.distance;
        }
        self.state = EdgeState::Pull;
        // A stretch never pulls beyond 1.
        self.pull_distance = (self.pull_distance + delta).min(1.0);
        self.distance = self.pull_distance.max(0.0);
        self.velocity = 0.0;
        if self.distance == 0.0 {
            self.state = EdgeState::Idle;
        }
    }

    /// `onPullDistance`: pulls by `delta` — negative releases the pull —
    /// and returns how much of it the effect consumed: never past zero.
    pub(super) fn on_pull_distance(&mut self, delta: f64) -> f64 {
        let consumed = (delta + self.distance).max(0.0) - self.distance;
        if consumed == 0.0 && self.distance == 0.0 {
            return 0.0;
        }
        self.on_pull(consumed);
        consumed
    }

    /// `onRelease`: a pulled stretch relaxes from rest on the spring.
    pub(super) fn on_release(&mut self, now: Instant) {
        self.pull_distance = 0.0;
        if self.state != EdgeState::Pull {
            return;
        }
        self.state = EdgeState::Recede;
        self.velocity = 0.0;
        self.stepped_at = Some(now);
    }

    /// `onAbsorb`: a fling reaching the edge at `speed`, in logical units
    /// per second, kicks the spring at `ON_ABSORB_VELOCITY_ADJUSTMENT`
    /// times it.
    pub(super) fn on_absorb(&mut self, speed: f64, config: &EdgeStretch, now: Instant) {
        self.state = EdgeState::Recede;
        self.velocity = speed * config.absorb_velocity_scale();
        self.stepped_at = Some(now);
    }

    /// `ScrollView`'s `ACTION_DOWN` on an unfinished edge,
    /// `onPullDistance(0, …)`: a touch holds the stretch where it is.
    fn catch(&mut self) {
        if !self.is_finished() {
            self.on_pull_distance(0.0);
        }
    }

    /// `draw`'s per-frame step for an edge `dimension` long: `updateSpring`
    /// while relaxing, then the end of a relaxed run. Returns whether the
    /// shown distance changed.
    pub(super) fn step(&mut self, now: Instant, dimension: f64, config: &EdgeStretch) -> bool {
        let before = self.distance;
        if self.state == EdgeState::Recede {
            self.update_spring(now, dimension, config);
        }
        if self.state == EdgeState::Recede && self.distance == 0.0 && self.velocity == 0.0 {
            self.state = EdgeState::Idle;
        }
        self.distance.to_bits() != before.to_bits()
    }

    /// `updateSpring`: steps the underdamped spring from the last step to
    /// `now`, in closed form, and finishes a run close to rest linearly.
    fn update_spring(&mut self, now: Instant, dimension: f64, config: &EdgeStretch) {
        let stepped_at = self
            .stepped_at
            .expect("a relaxing stretch records when it started");
        let dt = now.saturating_duration_since(stepped_at).as_secs_f64();
        // At least 1 ms between steps.
        if dt < 0.001 {
            return;
        }
        self.stepped_at = Some(now);
        if self.velocity.abs() <= config.linear_take_over_velocity()
            && (self.distance * dimension).abs() < config.linear_take_over_distance()
            && java_signum(self.velocity) == -java_signum(self.distance)
        {
            // Close to rest the spring crawls: finish at a constant speed.
            self.velocity =
                f64::from(java_signum(self.velocity)) * config.linear_take_over_velocity();
            let target = self.distance + self.velocity * dt / dimension;
            if java_signum(target) == java_signum(self.distance) {
                self.distance = target;
            } else {
                self.distance = 0.0;
                self.velocity = 0.0;
            }
            return;
        }
        let omega = config.natural_frequency();
        let zeta = config.damping_ratio();
        let damped = omega * zeta.mul_add(-zeta, 1.0).sqrt();
        let cos_coeff = self.distance * dimension;
        let sin_coeff = (zeta * omega).mul_add(cos_coeff, self.velocity) / damped;
        let decay = (-zeta * omega * dt).exp();
        let (sin, cos) = (damped * dt).sin_cos();
        let displacement = decay * cos_coeff.mul_add(cos, sin_coeff * sin);
        let velocity = (displacement * -omega).mul_add(
            zeta,
            decay * (-damped * cos_coeff).mul_add(sin, damped * sin_coeff * cos),
        );
        self.distance = displacement / dimension;
        self.velocity = velocity;
        if self.distance > 1.0 {
            self.distance = 1.0;
            self.velocity = 0.0;
        }
        // `isAtEquilibrium`: never stretch the other way, and stop once
        // close enough.
        let displacement = self.distance * dimension;
        if displacement < 0.0
            || (self.velocity.abs() < config.rest_velocity()
                && displacement < config.rest_distance())
        {
            self.distance = 0.0;
            self.velocity = 0.0;
        }
    }

    /// `dampStretchVector(mDistance)`: the stretch intensity the shown
    /// distance draws with — linear in the pull plus an exponential share
    /// that saturates over `SCROLL_DIST_AFFECTED_BY_EXP_STRETCH`.
    pub(super) fn intensity(&self, config: &EdgeStretch) -> f64 {
        let overscroll = self.distance.clamp(-1.0, 1.0);
        let sign = if overscroll > 0.0 { 1.0 } else { -1.0 };
        let overscroll = overscroll.abs();
        let linear = config.linear_intensity() * overscroll;
        let scalar = core::f64::consts::E / config.exponential_distance();
        let exponential = config.exponential_intensity() * (1.0 - (-overscroll * scalar).exp());
        sign * (linear + exponential)
    }
}

/// Which edge of an axis: the one at offset 0 (top, left) or the one at
/// the maximum offset (bottom, right).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Edge {
    Leading,
    Trailing,
}

/// A fling that reaches an edge `at`, moving at `speed` there: the edge
/// absorbs it on the first step at or after `at`, as `ScrollView`'s
/// `computeScroll` absorbs on the frame that finds the offset past it.
#[derive(Debug, Clone, Copy)]
pub(super) struct EdgeImpact {
    pub(super) at: Instant,
    pub(super) edge: Edge,
    pub(super) speed: f64,
}

/// One axis's two edge effects — `ScrollView`'s `mEdgeGlowTop` and
/// `mEdgeGlowBottom`.
#[derive(Debug, Clone)]
pub(super) struct AxisStretch {
    leading: EdgeEffect,
    trailing: EdgeEffect,
    /// A running fling's arrival at an edge, absorbed once it is reached.
    impact: Option<EdgeImpact>,
}

impl AxisStretch {
    const IDLE: Self = Self {
        leading: EdgeEffect::IDLE,
        trailing: EdgeEffect::IDLE,
        impact: None,
    };

    const fn edge_mut(&mut self, edge: Edge) -> &mut EdgeEffect {
        match edge {
            Edge::Leading => &mut self.leading,
            Edge::Trailing => &mut self.trailing,
        }
    }

    /// `ScrollView.onTouchEvent`'s `ACTION_MOVE` along one axis: a drag
    /// moving the offset at `offset` by `delta` — the finger's travel,
    /// negated — on an axis whose extents are `0..=max` and whose viewport
    /// is `dimension` long. A drag back from a stretched edge first relaxes
    /// it; the rest scrolls, clamped; a drag pushing past an edge pulls its
    /// stretch — by the whole delta, as `ScrollView` pulls — and releases
    /// the opposite one. Returns the new offset.
    ///
    /// An axis with nothing to scroll does not stretch: `ScrollView`'s
    /// default `OVER_SCROLL_IF_CONTENT_SCROLLS` overscrolls only when
    /// `getScrollRange() > 0`.
    pub(super) fn drag(
        &mut self,
        offset: f64,
        delta: f64,
        max: f64,
        dimension: f64,
        now: Instant,
    ) -> f64 {
        if max <= 0.0 || dimension <= 0.0 {
            return offset;
        }
        let mut delta = delta;
        if delta < 0.0 && self.trailing.distance() != 0.0 {
            delta = dimension.mul_add(-self.trailing.on_pull_distance(delta / dimension), delta);
        } else if delta > 0.0 && self.leading.distance() != 0.0 {
            delta = dimension.mul_add(self.leading.on_pull_distance(-delta / dimension), delta);
        }
        let pulled_to = offset + delta;
        if delta != 0.0 {
            if pulled_to < 0.0 {
                self.leading.on_pull_distance(-delta / dimension);
                if !self.trailing.is_finished() {
                    self.trailing.on_release(now);
                }
            } else if pulled_to > max {
                self.trailing.on_pull_distance(delta / dimension);
                if !self.leading.is_finished() {
                    self.leading.on_release(now);
                }
            }
        }
        pulled_to.clamp(0.0, max)
    }

    /// `ScrollView`'s `ACTION_UP` along one axis, the offset at `offset`
    /// inside `0..=max` released at `velocity` — the offset's, at least
    /// `min_velocity` to fling: returns whether the release flings
    /// (`flingWithNestedDispatch`'s `canFling`). A fast release that cannot
    /// fling — it pushes against the edge it sits on — is absorbed by
    /// whichever edge shows a stretch, top first; then both edges let go
    /// (`endDrag`).
    pub(super) fn release(
        &mut self,
        offset: f64,
        velocity: f64,
        max: f64,
        min_velocity: f64,
        config: &EdgeStretch,
        now: Instant,
    ) -> bool {
        let fast = velocity.abs() >= min_velocity;
        let flings = fast && (offset > 0.0 || velocity > 0.0) && (offset < max || velocity < 0.0);
        if fast && !flings {
            if !self.leading.is_finished() {
                self.leading.on_absorb(-velocity, config, now);
            } else if !self.trailing.is_finished() {
                self.trailing.on_absorb(velocity, config, now);
            }
        }
        self.leading.on_release(now);
        self.trailing.on_release(now);
        flings
    }

    /// Arms the absorb of a fling's arrival at an edge.
    pub(super) const fn expect_impact(&mut self, impact: EdgeImpact) {
        self.impact = Some(impact);
    }

    /// Advances both edges to `now` — absorbing an impact reached by then
    /// — and returns whether the shown stretch changed.
    pub(super) fn step(&mut self, now: Instant, dimension: f64, config: &EdgeStretch) -> bool {
        if let Some(impact) = self.impact.filter(|impact| now >= impact.at) {
            self.impact = None;
            self.edge_mut(impact.edge)
                .on_absorb(impact.speed, config, now);
        }
        let leading = self.leading.step(now, dimension, config);
        let trailing = self.trailing.step(now, dimension, config);
        leading || trailing
    }

    /// Whether an edge still stretches or a fling is still to reach one.
    pub(super) fn is_active(&self) -> bool {
        !self.leading.is_finished() || !self.trailing.is_finished() || self.impact.is_some()
    }

    /// The axis's stretch intensity: the leading edge's positive, the
    /// trailing edge's negative — `ScrollView.draw` rotates the bottom
    /// edge's canvas by 180°, which flips its vector — summed as
    /// `StretchEffect::mergeWith` sums the stretches a node receives.
    pub(super) fn intensity(&self, config: &EdgeStretch) -> f64 {
        let leading = if self.leading.distance() == 0.0 {
            0.0
        } else {
            self.leading.intensity(config)
        };
        let trailing = if self.trailing.distance() == 0.0 {
            0.0
        } else {
            self.trailing.intensity(config)
        };
        leading - trailing
    }
}

/// A scroll view's stretch on both axes, carried by the touch drag that
/// holds the scroll view and then by its `ScrollMotion`, which outlives the
/// fling that owns the offset.
#[derive(Debug, Clone)]
pub(super) struct ScrollStretch {
    pub(super) config: EdgeStretch,
    pub(super) x: AxisStretch,
    pub(super) y: AxisStretch,
}

impl ScrollStretch {
    pub(super) const fn new(config: EdgeStretch) -> Self {
        Self {
            config,
            x: AxisStretch::IDLE,
            y: AxisStretch::IDLE,
        }
    }

    /// A touch down holds every unfinished edge where it is, and stops the
    /// fling that was to reach an edge — its absorb never comes.
    pub(super) fn catch(&mut self) {
        self.cancel_impacts();
        for axis in [&mut self.x, &mut self.y] {
            axis.leading.catch();
            axis.trailing.catch();
        }
    }

    /// The fling that was to reach an edge stopped short of it: drops the
    /// absorbs it armed.
    pub(super) const fn cancel_impacts(&mut self) {
        self.x.impact = None;
        self.y.impact = None;
    }

    /// `endDrag`'s `onRelease` on every edge, with no fling: a held edge
    /// relaxes from rest.
    pub(super) fn release(&mut self, now: Instant) {
        for axis in [&mut self.x, &mut self.y] {
            axis.leading.on_release(now);
            axis.trailing.on_release(now);
        }
    }

    /// Advances both axes to `now` on a viewport `width` × `height`;
    /// returns whether the shown stretch changed.
    pub(super) fn step(&mut self, now: Instant, width: f64, height: f64) -> bool {
        let x = self.x.step(now, width, &self.config);
        let y = self.y.step(now, height, &self.config);
        x || y
    }

    pub(super) fn is_active(&self) -> bool {
        self.x.is_active() || self.y.is_active()
    }

    /// The stretch intensities `(x, y)` HWUI's `StretchEffect` receives:
    /// positive stretches away from the leading edge, negative away from
    /// the trailing one.
    pub(super) fn vector(&self) -> kurbo::Vec2 {
        kurbo::Vec2::new(
            self.x.intensity(&self.config),
            self.y.intensity(&self.config),
        )
    }
}

/// The stretch direction HWUI draws for `vector`, or `None` when the
/// stretch is empty (`StretchEffect::isEmpty`: both intensities within
/// `NON_ZERO_EPSILON` of zero) — HWUI then neither composes the node as a
/// layer (`StretchEffect::requiresLayer`) nor runs the shader.
pub(super) fn shown_stretch(vector: kurbo::Vec2) -> Option<kurbo::Vec2> {
    (vector.x.abs() > NON_ZERO_EPSILON || vector.y.abs() > NON_ZERO_EPSILON).then_some(vector)
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    /// The viewport length every case runs on, in logical units at density
    /// 1 — `EdgeEffect`'s `mHeight`.
    const HEIGHT: f64 = 600.0;
    /// The frame interval the spring is stepped at.
    const FRAME: Duration = Duration::from_millis(16);

    fn config() -> EdgeStretch {
        EdgeStretch::android(1.0)
    }

    fn close(actual: f64, expected: f64, what: &str) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "{what}: {actual} vs AOSP {expected}"
        );
    }

    /// A drag past the top edge pulls `mDistance` by the drag over the
    /// viewport (`ScrollView`'s `onPullDistance(-deltaY / getHeight())`),
    /// and the drawn intensity is `dampStretchVector(mDistance)` =
    /// `0.016·d + 0.016·(1 − e^(−d·e/0.33))`:
    /// d = 0.1 → 0.01057927172144907, d = 0.2 → 0.016119335902422205,
    /// d = 0.15 → 0.013749354287135923, d = 1 → 0.03199576608820807.
    #[test]
    fn a_pull_stretches_by_aosps_damped_intensity() {
        let config = config();
        let now = Instant::now();
        let mut axis = AxisStretch::IDLE;
        // 60 units of finger travel down at the top: the offset stays on
        // the edge and the top edge pulls 0.1.
        assert!(axis.drag(0.0, -60.0, 1_000.0, HEIGHT, now).abs() < f64::EPSILON);
        close(axis.leading.distance(), 0.1, "pull distance");
        close(
            axis.intensity(&config),
            0.010_579_271_721_449_07,
            "intensity at 0.1",
        );
        // Another 60 accumulate.
        assert!(axis.drag(0.0, -60.0, 1_000.0, HEIGHT, now).abs() < f64::EPSILON);
        close(
            axis.intensity(&config),
            0.016_119_335_902_422_205,
            "intensity at 0.2",
        );
        // Dragging back 30 relaxes the stretch first and scrolls nothing.
        close(
            axis.drag(0.0, 30.0, 1_000.0, HEIGHT, now),
            0.0,
            "nothing scrolls",
        );
        close(axis.leading.distance(), 0.15, "relaxed pull");
        close(
            axis.intensity(&config),
            0.013_749_354_287_135_923,
            "intensity at 0.15",
        );
        // Dragging back further spends the rest of the stretch, then scrolls.
        let offset = axis.drag(0.0, 100.0, 1_000.0, HEIGHT, now);
        close(offset, 10.0, "the remainder scrolls");
        assert!(axis.leading.distance().abs() < f64::EPSILON);
        // A pull never exceeds 1.
        axis.drag(0.0, -10.0 * HEIGHT, 1_000.0, HEIGHT, now);
        close(axis.leading.distance(), 1.0, "pull cap");
        close(
            axis.intensity(&config),
            0.031_995_766_088_208_07,
            "intensity at 1",
        );

        // The bottom edge stretches the other way.
        let mut axis = AxisStretch::IDLE;
        close(
            axis.drag(1_000.0, 60.0, 1_000.0, HEIGHT, now),
            1_000.0,
            "bottom offset",
        );
        close(
            axis.intensity(&config),
            -0.010_579_271_721_449_07,
            "bottom intensity",
        );
        // An axis with nothing to scroll never stretches.
        let mut axis = AxisStretch::IDLE;
        assert!(axis.drag(0.0, -60.0, 0.0, HEIGHT, now).abs() < f64::EPSILON);
        assert!(!axis.is_active());
    }

    /// `StretchEffect::isEmpty`: a stretch within `NON_ZERO_EPSILON` of
    /// zero on both axes draws nothing; past it on either axis, the vector
    /// reaches the shader whole.
    #[test]
    fn a_stretch_under_hwuis_epsilon_draws_nothing() {
        assert_eq!(
            shown_stretch(kurbo::Vec2::new(0.000_01, -0.000_04)),
            None,
            "under NON_ZERO_EPSILON the stretch is empty"
        );
        let shown = kurbo::Vec2::new(0.000_01, -0.000_05);
        assert_eq!(shown_stretch(shown), Some(shown));
        let top = kurbo::Vec2::new(0.0, 0.016_119_335_902_422_205);
        assert_eq!(shown_stretch(top), Some(top));
    }

    /// `onRelease` from a 0.2 pull, stepped by `updateSpring` every 16 ms:
    /// the underdamped spring `e^(−ζω·t)·(x0·cos(ωd·t) + (ζω·x0 + v0)/ωd ·
    /// sin(ωd·t))` with ω = 24.657, ζ = 0.98, x0 = 0.2·600, v0 = 0; the
    /// linear take-over at 200 px/s from frame 12, once under 8 px and
    /// slower than 200 px/s; at rest on frame 14.
    #[test]
    fn a_release_relaxes_on_aosps_spring() {
        let config = config();
        let start = Instant::now();
        let mut edge = EdgeEffect::IDLE;
        edge.on_pull_distance(0.2);
        edge.on_release(start);
        let expected = [
            (1, 0.187_927_311_414_835_07, -792.187_150_035_085_3),
            (2, 0.162_246_889_005_032_95, -1_073.026_294_391_795),
            (3, 0.133_034_182_878_557_84, -1_087.820_805_105_874_6),
            (4, 0.105_317_114_930_570_12, -978.249_019_039_838),
        ];
        let mut frame = 0_u32;
        for (at, distance, velocity) in expected {
            while frame < at {
                frame += 1;
                edge.step(start + FRAME * frame, HEIGHT, &config);
            }
            close(edge.distance, distance, "released distance");
            assert!(
                (edge.velocity - velocity).abs() < 1e-6,
                "frame {at}: velocity {} vs AOSP {velocity}",
                edge.velocity
            );
        }
        while frame < 12 {
            frame += 1;
            edge.step(start + FRAME * frame, HEIGHT, &config);
        }
        close(edge.distance, 0.007_164_613_238_589_985, "linear take-over");
        close(edge.velocity, -200.0, "take-over speed");
        frame += 1;
        edge.step(start + FRAME * frame, HEIGHT, &config);
        assert!(!edge.is_finished(), "still relaxing on frame 13");
        frame += 1;
        assert!(edge.step(start + FRAME * frame, HEIGHT, &config));
        assert!(edge.is_finished(), "at rest on frame 14");
        assert!(edge.distance.abs() < f64::EPSILON);
    }

    /// `onAbsorb(2000)`: the spring starts on the edge at 13 × 2000 px/s
    /// (`ON_ABSORB_VELOCITY_ADJUSTMENT`), the same closed form with x0 = 0,
    /// v0 = 26000; it peaks on frame 3 at 0.6461267803767068 and rests on
    /// frame 20.
    #[test]
    fn an_absorbed_fling_kicks_aosps_spring() {
        let config = config();
        let start = Instant::now();
        let mut edge = EdgeEffect::IDLE;
        edge.on_absorb(2_000.0, &config, start);
        let expected = [
            (1, 0.470_530_927_801_249, 10_786.738_325_857_179),
            (2, 0.637_339_368_396_654_8, 2_611.400_446_144_056_3),
            (3, 0.646_126_780_376_706_8, -1_441.056_701_715_662_7),
            (4, 0.581_045_045_389_956_9, -3_157.124_415_621_755_4),
        ];
        let mut frame = 0_u32;
        for (at, distance, velocity) in expected {
            while frame < at {
                frame += 1;
                edge.step(start + FRAME * frame, HEIGHT, &config);
            }
            close(edge.distance, distance, "absorbed distance");
            assert!(
                (edge.velocity - velocity).abs() < 1e-6,
                "frame {at}: velocity {} vs AOSP {velocity}",
                edge.velocity
            );
        }
        while frame < 19 {
            frame += 1;
            edge.step(start + FRAME * frame, HEIGHT, &config);
            assert!(!edge.is_finished(), "still moving on frame {frame}");
        }
        edge.step(start + FRAME * 20, HEIGHT, &config);
        assert!(edge.is_finished(), "at rest on frame 20");
    }

    /// A fling arriving at an edge is absorbed on the first step at or
    /// after its arrival, and a touch down holds an unfinished edge.
    #[test]
    fn an_impact_is_absorbed_on_arrival_and_a_catch_holds_it() {
        let config = config();
        let start = Instant::now();
        let mut axis = AxisStretch::IDLE;
        axis.expect_impact(EdgeImpact {
            at: start + Duration::from_millis(10),
            edge: Edge::Trailing,
            speed: 2_000.0,
        });
        assert!(axis.is_active());
        assert!(!axis.step(start, HEIGHT, &config), "not there yet");
        axis.step(start + FRAME, HEIGHT, &config);
        axis.step(start + FRAME * 2, HEIGHT, &config);
        // Absorbed at 16 ms, stepped once 16 ms later: AOSP's frame 1.
        close(axis.trailing.distance(), 0.470_530_927_801_249, "absorbed");
        assert!(axis.intensity(&config) < 0.0, "the trailing edge stretches");
        let mut stretch = ScrollStretch::new(config);
        stretch.y = axis;
        stretch.catch();
        let held = stretch.vector();
        stretch.step(start + FRAME * 10, 400.0, HEIGHT);
        assert_eq!(stretch.vector(), held, "a caught stretch does not move");
    }
}
