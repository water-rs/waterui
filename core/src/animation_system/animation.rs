//! # `WaterUI` Animation System
//!
//! A reactive animation system that seamlessly integrates with `WaterUI`'s reactive state management.
//!
//! ## Overview
//!
//! The `WaterUI` animation system leverages the reactive framework to create smooth, declarative
//! animations that automatically run when reactive values change. By attaching animation metadata
//! to reactive values through convenient extension methods, the system can intelligently
//! determine how to animate between different states without requiring explicit animation code.
//!
//! ```text
//! ┌───────────────────┐      ┌───────────────────┐      ┌───────────────────┐
//! │  Reactive Values  │─────>│ Change Propagation│─────>│  Animation System │
//! │  (Binding/Compute)│      │ (With Animations) │      │  (Renderer)       │
//! └───────────────────┘      └───────────────────┘      └───────────────────┘
//! ```
//!
//! ## Core Concepts
//!
//! ### Animation Extension Methods
//!
//! `WaterUI` provides convenient extension methods on all reactive types to easily attach
//! animation configurations:
//!
//! ```rust
//! use waterui_core::{animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let opacity: nami::Binding<f32> = binding(1.0);
//!
//! // Use the .animated() method to apply the system-default animation
//! let _animated_opacity = opacity.animated();
//!
//! // Or attach a specific animation with the generic metadata method
//! let faded: nami::Binding<f32> = binding(0.0);
//! let _custom_animated = faded.with(Animation::ease_in_out(Duration::from_millis(300)));
//! ```
//!
//! The system supports two native primitives:
//!
//! - **`Bezier`**: Timed interpolation with cubic bezier control points
//! - **`Spring`**: Physics-based animation with configurable stiffness and damping
//!
//! Convenience constructors (`linear`, `ease_in`, `ease_out`, `ease_in_out`) map to `Bezier`.
//!
//! ### Integration with UI Components
//!
//! UI components automatically respect animation metadata when rendering:
//!
//! ```rust
//! use waterui_core::{animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let scale: nami::Binding<f32> = binding(1.0);
//!
//! // Three different ways to animate properties:
//!
//! // 1. Default animation (uses system defaults)
//! let _view1_scale = scale.animated();
//!
//! // 2. Custom animation using a convenience constructor
//! let expanded: nami::Binding<f32> = binding(2.0);
//! let _view2_scale = expanded.with(Animation::ease_in_out(Duration::from_millis(300)));
//!
//! // 3. Spring animation using the convenience constructor
//! let bouncing: nami::Binding<f32> = binding(0.5);
//! let _view3_scale = bouncing.with(Animation::spring(100.0, 10.0));
//! ```
//!
//! ## Animation Pipeline
//!
//! 1. **Reactive Setup**: Reactive values are wrapped with animation metadata using extension methods
//! 2. **State Change**: When the underlying value changes, the animation information is preserved
//! 3. **Propagation**: The change and animation details are propagated through the reactive system
//! 4. **Value Interpolation**: The renderer calculates intermediate values based on animation type
//! 5. **Rendering**: The UI is continuously updated with interpolated values until animation completes
//!
//! ## Advanced Features
//!
//! ### Animation Choreography
//!
//! Complex animations can be created by coordinating multiple animated values:
//!
//! ```rust
//! use waterui_core::{animation::Animation, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let opacity: nami::Binding<f32> = binding(0.0);
//! let position: nami::Binding<(f32, f32)> = binding((0.0, 0.0));
//!
//! // Create a choreographed animation sequence
//! let animated_opacity = opacity.with(Animation::ease_in_out(Duration::from_millis(300)));
//!
//! // Position animates with a spring physics model
//! let animated_position = position.with(Animation::spring(100.0, 10.0));
//!
//! // Both animated values can be used in views
//! // The UI framework will automatically handle the animation timing
//! drop((animated_opacity, animated_position));
//! ```
//!
//! ### Composition with Other Reactive Features
//!
//! Animation metadata seamlessly composes with other reactive features:
//!
//! ```rust
//! use waterui_core::{animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let count: nami::Binding<i32> = binding(0i32);
//! let value1: nami::Binding<i32> = binding(1i32);
//! let value2: nami::Binding<i32> = binding(2i32);
//!
//! // Combine mapping and animation
//! let opacity = count
//!     .map(|n: i32| if n > 5 { 1.0 } else { 0.5 })
//!     .animated();  // Apply animation to the mapped result
//!
//! // Combine multiple reactive values with animation
//! let combined = value1
//!     .zip(&value2)
//!     .map(|(a, b)| a + b)
//!     .with(Animation::ease_in_out(Duration::from_millis(250)));
//!
//! drop((opacity, combined)); // Prevent unused variable warnings
//! ```
//!

use core::time::Duration;

use crate::easing::{
    EasingCurve, Interpolatable, Lanes, Spring, Timing, bezier_is_valid, cubic_bezier, lanes_of,
};

/// SwiftUI-style animation protocol.
///
/// Types expose an animatable representation (`AnimatableData`) that can be
/// linearly interpolated by the animation system.
pub trait Animatable: Clone {
    /// Interpolatable payload used for frame-to-frame value blending.
    type AnimatableData: Interpolatable;

    /// Exports the value to its animatable representation.
    fn animatable_data(&self) -> Self::AnimatableData;

    /// Reconstructs the value from animatable data.
    fn from_animatable_data(data: Self::AnimatableData) -> Self;
}

impl Animatable for f32 {
    type AnimatableData = Self;

    fn animatable_data(&self) -> Self::AnimatableData {
        *self
    }

    fn from_animatable_data(data: Self::AnimatableData) -> Self {
        data
    }
}

impl Animatable for f64 {
    type AnimatableData = Self;

    fn animatable_data(&self) -> Self::AnimatableData {
        *self
    }

    fn from_animatable_data(data: Self::AnimatableData) -> Self {
        data
    }
}

impl<A: Animatable, B: Animatable> Animatable for (A, B) {
    type AnimatableData = (A::AnimatableData, B::AnimatableData);

    fn animatable_data(&self) -> Self::AnimatableData {
        (self.0.animatable_data(), self.1.animatable_data())
    }

    fn from_animatable_data(data: Self::AnimatableData) -> Self {
        (
            A::from_animatable_data(data.0),
            B::from_animatable_data(data.1),
        )
    }
}

impl<A: Animatable, B: Animatable, C: Animatable> Animatable for (A, B, C) {
    type AnimatableData = (A::AnimatableData, B::AnimatableData, C::AnimatableData);

    fn animatable_data(&self) -> Self::AnimatableData {
        (
            self.0.animatable_data(),
            self.1.animatable_data(),
            self.2.animatable_data(),
        )
    }

    fn from_animatable_data(data: Self::AnimatableData) -> Self {
        (
            A::from_animatable_data(data.0),
            B::from_animatable_data(data.1),
            C::from_animatable_data(data.2),
        )
    }
}

impl<A: Animatable, B: Animatable, C: Animatable, D: Animatable> Animatable for (A, B, C, D) {
    type AnimatableData = (
        A::AnimatableData,
        B::AnimatableData,
        C::AnimatableData,
        D::AnimatableData,
    );

    fn animatable_data(&self) -> Self::AnimatableData {
        (
            self.0.animatable_data(),
            self.1.animatable_data(),
            self.2.animatable_data(),
            self.3.animatable_data(),
        )
    }

    fn from_animatable_data(data: Self::AnimatableData) -> Self {
        (
            A::from_animatable_data(data.0),
            B::from_animatable_data(data.1),
            C::from_animatable_data(data.2),
            D::from_animatable_data(data.3),
        )
    }
}

impl<T: Animatable + Copy, const N: usize> Animatable for [T; N]
where
    T::AnimatableData: Copy,
{
    type AnimatableData = [T::AnimatableData; N];

    fn animatable_data(&self) -> Self::AnimatableData {
        core::array::from_fn(|index| self[index].animatable_data())
    }

    fn from_animatable_data(data: Self::AnimatableData) -> Self {
        core::array::from_fn(|index| T::from_animatable_data(data[index]))
    }
}

/// A running animation of a track, in lanes.
#[derive(Debug, Clone)]
struct ActiveTrack<T: Animatable> {
    timing: Timing,
    elapsed: Duration,
    /// The lanes the animation started from.
    from: Lanes,
    /// The lane velocities, per second, the animation started with.
    velocity: Lanes,
    /// The target's lanes.
    target: Lanes,
    /// The lane velocities at the last advance: what a retarget carries.
    rate: Lanes,
    to: T,
}

/// Shared animation timeline state for values implementing [`Animatable`].
///
/// The track evaluates its animation lane by lane through the shared
/// evaluator, as the engine does. A new target set while an animation runs
/// starts from the current value with the current velocity, so a spring
/// retargeted mid-flight stays continuous in position and velocity.
#[derive(Debug, Clone)]
pub struct AnimationTrack<T: Animatable> {
    current: T,
    active: Option<ActiveTrack<T>>,
}

impl<T: Animatable> AnimationTrack<T> {
    /// Create a track seeded with an initial value.
    #[must_use]
    pub const fn new(initial: T) -> Self {
        Self {
            current: initial,
            active: None,
        }
    }

    /// Current sampled value.
    #[must_use]
    pub fn value(&self) -> T {
        self.current.clone()
    }

    /// Replace the target value.
    ///
    /// If animation metadata is absent, or the animation is a zero-duration
    /// curve, the value is applied immediately. Otherwise the animation
    /// starts from the current value, carrying the velocity of the animation
    /// it interrupts: a spring continues smoothly toward the new target,
    /// while a curve runs its own shape from the current value.
    pub fn set_target(&mut self, target: T, animation: Option<Animation>) {
        let timing = animation.map(|animation| animation.validated().timing());
        match timing {
            Some(Timing::Curve { duration, .. }) if duration <= 0.0 => self.snap(target),
            None => self.snap(target),
            Some(timing) => {
                let lanes = <T::AnimatableData as Interpolatable>::LANES;
                let velocity = self
                    .active
                    .take()
                    .map_or_else(|| Lanes::from_elem(0.0, lanes), |active| active.rate);
                self.active = Some(ActiveTrack {
                    timing,
                    elapsed: Duration::ZERO,
                    from: lanes_of(&self.current.animatable_data()),
                    velocity,
                    target: lanes_of(&target.animatable_data()),
                    rate: Lanes::from_elem(0.0, lanes),
                    to: target,
                });
            }
        }
    }

    fn snap(&mut self, target: T) {
        self.current = target;
        self.active = None;
    }

    /// Advance by a frame delta.
    ///
    /// Returns `true` while an animation remains active after advancement.
    pub fn advance(&mut self, delta: Duration) -> bool {
        let Some(active) = self.active.as_mut() else {
            return false;
        };

        active.elapsed = active.elapsed.saturating_add(delta);
        let mut position = Lanes::from_elem(0.0, active.from.len());
        let done = active.timing.sample_lanes(
            &active.from,
            &active.velocity,
            &active.target,
            active.elapsed.as_secs_f64(),
            &mut position,
            &mut active.rate,
        );
        if done {
            let target = active.to.clone();
            self.snap(target);
            false
        } else {
            self.current = T::from_animatable_data(T::AnimatableData::from_lanes(&position));
            true
        }
    }

    /// Whether this track has an active animation.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active.is_some()
    }
}

/// Default duration for timed animations.
const DEFAULT_TIMED_DURATION: Duration = Duration::from_millis(250);

/// The shared evaluator's spring for a stiffness and damping coefficient on
/// a unit mass.
fn physical(stiffness: f32, damping: f32) -> Spring {
    Spring::from_physics(f64::from(stiffness), f64::from(damping))
}

/// Whether spring parameters describe a physical spring that settles: a
/// finite stiffness and damping, both greater than zero — exactly when
/// [`Spring::from_physics`] gives a [valid](Spring::is_valid) spring.
pub(super) const fn spring_is_valid(stiffness: f32, damping: f32) -> bool {
    stiffness.is_finite() && stiffness > 0.0 && damping.is_finite() && damping > 0.0
}

/// An enumeration representing different types of animations
///
/// This enum exposes two native animation primitives:
/// - `Bezier`: Timed animations represented by cubic bezier control points
/// - `Spring`: A physical spring with configurable stiffness and damping,
///   running in real seconds until it settles
///
/// Convenience constructors (`linear`, `ease_in`, etc.) all map to `Bezier`.
/// Every animation evaluates through the shared evaluator
/// ([`timing`](Self::timing)), the one the Cherenkov engine samples, so a
/// value animates identically whichever side draws it.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each animation primitive natively, variant by variant; a new primitive must fail their build instead of falling into a wildcard arm"
)]
pub enum Animation {
    /// Default animation behavior (uses system defaults)
    #[default]
    Default,
    /// Timed cubic bezier animation with control points (x1, y1, x2, y2)
    Bezier {
        /// Animation duration
        duration: Duration,
        /// First control point X (0.0 to 1.0)
        x1: f32,
        /// First control point Y
        y1: f32,
        /// Second control point X (0.0 to 1.0)
        x2: f32,
        /// Second control point Y
        y2: f32,
    },
    /// A damped spring on a unit mass, evolving in real seconds: the
    /// stiffness and damping coefficient `UISpringTimingParameters`,
    /// `CASpringAnimation` and Compose's `spring` take.
    ///
    /// It overshoots when `damping < 2√stiffness` and runs until it settles
    /// within [`SETTLE_TOLERANCE`](crate::easing::SETTLE_TOLERANCE); a
    /// retarget carries its velocity.
    Spring {
        /// Stiffness `k` on a unit mass: higher is faster. The natural
        /// angular frequency is `√k` radians per second.
        stiffness: f32,
        /// Damping coefficient `c`: higher bounces less; `2√k` is critically
        /// damped.
        damping: f32,
    },
}

nami::impl_constant!(Animation);

impl Animation {
    /// Creates a new Linear animation with the specified duration
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// let animation = Animation::linear(Duration::from_millis(300)); // 300ms
    /// let animation = Animation::linear(Duration::from_secs(1)); // 1 second
    /// ```
    #[must_use]
    pub const fn linear(duration: Duration) -> Self {
        Self::Bezier {
            duration,
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        }
    }

    /// Creates a new ease-in animation with the specified duration
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// let animation = Animation::ease_in(Duration::from_millis(300)); // 300ms
    /// let animation = Animation::ease_in(Duration::from_secs(1)); // 1 second
    /// ```
    #[must_use]
    pub const fn ease_in(duration: Duration) -> Self {
        Self::Bezier {
            duration,
            x1: 0.42,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        }
    }

    /// Creates a new ease-out animation with the specified duration
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// let animation = Animation::ease_out(Duration::from_millis(300)); // 300ms
    /// let animation = Animation::ease_out(Duration::from_secs(1)); // 1 second
    /// ```
    #[must_use]
    pub const fn ease_out(duration: Duration) -> Self {
        Self::Bezier {
            duration,
            x1: 0.0,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        }
    }

    /// Creates a new ease-in-out animation with the specified duration
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// let animation = Animation::ease_in_out(Duration::from_millis(300)); // 300ms
    /// let animation = Animation::ease_in_out(Duration::from_secs(1)); // 1 second
    /// ```
    #[must_use]
    pub const fn ease_in_out(duration: Duration) -> Self {
        Self::Bezier {
            duration,
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        }
    }

    /// Creates a physical spring with the given stiffness and damping
    /// coefficient on a unit mass.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// // ω₀ = 10 rad/s, damping ratio 0.5: it overshoots, then settles.
    /// let animation = Animation::spring(100.0, 10.0);
    /// assert!(animation.progress(Duration::from_millis(400)) > 1.0);
    /// assert!(animation.is_complete(animation.duration()));
    /// ```
    ///
    /// Stiffness and damping must be finite and greater than zero: an
    /// undamped spring never settles. A debug build asserts this; the
    /// constructor stays plain data, and a consumer calls
    /// [`validated`](Self::validated) to turn an invalid animation into the
    /// default one.
    #[must_use]
    pub const fn spring(stiffness: f32, damping: f32) -> Self {
        debug_assert!(
            spring_is_valid(stiffness, damping),
            "Animation::spring requires finite stiffness > 0 and finite damping > 0"
        );
        Self::Spring { stiffness, damping }
    }

    /// Creates a new custom cubic bezier animation
    ///
    /// Control points define the shape of the easing curve.
    /// Standard curves can be created with these control points:
    /// - Linear: (0.0, 0.0, 1.0, 1.0)
    /// - Ease-in: (0.42, 0.0, 1.0, 1.0)
    /// - Ease-out: (0.0, 0.0, 0.58, 1.0)
    /// - Ease-in-out: (0.42, 0.0, 0.58, 1.0)
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// // Custom bounce-like curve
    /// let animation = Animation::bezier(Duration::from_millis(400), 0.25, 0.1, 0.25, 1.0);
    /// ```
    ///
    /// Every control point must be finite and `x1` and `x2` must lie in the
    /// normalized `[0, 1]` range. A debug build asserts this; the constructor
    /// stays plain data, and a consumer calls [`validated`](Self::validated)
    /// to turn an invalid animation into the default one.
    #[must_use]
    pub const fn bezier(duration: Duration, x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        debug_assert!(
            bezier_is_valid(x1, y1, x2, y2),
            "Animation::bezier requires finite control points with x1/x2 in [0, 1]"
        );
        Self::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }
    }

    /// This animation when its parameters are valid, otherwise the default
    /// animation.
    ///
    /// A consumer calls it where it takes an `Animation` from app code. An
    /// invalid spring or bezier is a contract violation: a debug build panics
    /// and a release build logs it and uses [`Animation::Default`].
    #[must_use]
    pub fn validated(self) -> Self {
        let valid = match &self {
            Self::Default => true,
            Self::Bezier { x1, y1, x2, y2, .. } => bezier_is_valid(*x1, *y1, *x2, *y2),
            Self::Spring { stiffness, damping } => spring_is_valid(*stiffness, *damping),
        };
        if valid {
            self
        } else {
            crate::contract_violation!("invalid animation parameters: {self:?}");
            Self::Default
        }
    }

    /// How this animation moves, in the shared evaluator's terms: the
    /// timing curve over its duration, or the physical spring.
    ///
    /// The Cherenkov engine evaluates the same [`Timing`], which is what
    /// makes core and the engine sample an animation identically.
    #[must_use]
    pub fn timing(&self) -> Timing {
        match *self {
            Self::Default => Timing::Curve {
                curve: EasingCurve::EASE_IN_OUT.cubic_bezier(),
                duration: DEFAULT_TIMED_DURATION.as_secs_f64(),
            },
            Self::Bezier {
                duration,
                x1,
                y1,
                x2,
                y2,
            } => Timing::Curve {
                curve: cubic_bezier(x1, y1, x2, y2),
                duration: duration.as_secs_f64(),
            },
            Self::Spring { stiffness, damping } => Timing::Spring(physical(stiffness, damping)),
        }
    }

    /// How long the animation runs from rest to its target.
    ///
    /// A curve runs its duration. A spring runs until it settles, which
    /// depends on how far it moves; this is the time by which a unit step
    /// from rest has settled for good, so [`progress`](Self::progress) is
    /// exactly `1` from then on.
    #[must_use]
    pub fn duration(&self) -> Duration {
        match *self {
            Self::Default => DEFAULT_TIMED_DURATION,
            Self::Bezier { duration, .. } => duration,
            Self::Spring { stiffness, damping } => physical(stiffness, damping)
                .unit_settling_duration()
                .unwrap_or_else(|| {
                    crate::contract_violation!(
                        "a spring that never settles has no duration: {self:?}"
                    );
                    Duration::MAX
                }),
        }
    }

    /// The eased progress `elapsed` after a unit step from rest started:
    /// `0` at the start and exactly `1` once the animation is complete.
    ///
    /// A curve follows its timing over its duration. A spring follows its
    /// step response in real seconds and may overshoot (return values above
    /// `1` or below `0`) before it settles.
    #[must_use]
    pub fn progress(&self, elapsed: Duration) -> f64 {
        self.step(elapsed).0
    }

    /// The unit step `elapsed` in: its progress and whether it is done.
    fn step(&self, elapsed: Duration) -> (f64, bool) {
        let (mut position, mut rate) = ([0.0], [0.0]);
        let done = self.timing().sample_lanes(
            &[0.0],
            &[0.0],
            &[1.0],
            elapsed.as_secs_f64(),
            &mut position,
            &mut rate,
        );
        (position[0], done)
    }

    /// Interpolate between two values based on elapsed time.
    ///
    /// Each lane moves from `from` at rest to `to` exactly as an
    /// [`AnimationTrack`] does, and lands on `to` once the animation is
    /// complete.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    /// use core::time::Duration;
    ///
    /// let anim = Animation::ease_in_out(Duration::from_millis(300));
    /// let elapsed = Duration::from_millis(150); // halfway through
    ///
    /// let value = anim.interpolate(&0.0_f32, &100.0_f32, elapsed);
    /// // value is approximately 50.0, but eased
    /// ```
    pub fn interpolate<T: Animatable>(&self, from: &T, to: &T, elapsed: Duration) -> T {
        let from = lanes_of(&from.animatable_data());
        let target = lanes_of(&to.animatable_data());
        let at_rest = Lanes::from_elem(0.0, from.len());
        let (mut position, mut rate) = (at_rest.clone(), at_rest.clone());
        if self.timing().sample_lanes(
            &from,
            &at_rest,
            &target,
            elapsed.as_secs_f64(),
            &mut position,
            &mut rate,
        ) {
            return to.clone();
        }
        T::from_animatable_data(T::AnimatableData::from_lanes(&position))
    }

    /// Returns true if the animation is complete `elapsed` after a unit step
    /// from rest started.
    ///
    /// A curve is complete once its duration has passed, a spring once it
    /// has settled within
    /// [`SETTLE_TOLERANCE`](crate::easing::SETTLE_TOLERANCE).
    #[must_use]
    pub fn is_complete(&self, elapsed: Duration) -> bool {
        self.step(elapsed).1
    }
}

use nami::signal::WithMetadata;
use nami::watcher::Metadata;

/// Change metadata stating that the change must not animate.
///
/// A change that names no animation leaves the receiver to choose: a control
/// that owns its motion may still animate it with its own default. A change
/// carrying `NoAnimation` applies in the next frame on every backend,
/// whatever the receiver's default.
///
/// Attach it the way an animation is attached:
///
/// ```rust
/// use waterui_core::animation::NoAnimation;
/// use nami::{SignalExt, binding};
///
/// let is_on: nami::Binding<bool> = binding(false);
/// let _snapping = is_on.with(NoAnimation);
/// ```
///
/// Disabling wins over an animation attached anywhere else on the same
/// change: `value.animated().with(NoAnimation)` and
/// `value.with(NoAnimation).animated()` both snap. Read a change's metadata
/// with [`ChangeAnimation::of`], never by probing for this type directly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NoAnimation;

/// How a change asks to be animated: the three cases its metadata can state.
///
/// Every backend reads a change's animation through [`ChangeAnimation::of`],
/// so the rules — an attached [`NoAnimation`] wins over an attached
/// [`Animation`] — live in one place.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "a change either names an animation, names none, or disables animation; receivers must handle exactly these three"
)]
pub enum ChangeAnimation {
    /// The change animates with this animation.
    Animated(Animation),
    /// The change names no animation; the receiver decides.
    Unspecified,
    /// The change must not animate: it applies in the next frame.
    Disabled,
}

impl ChangeAnimation {
    /// Reads the animation case a change's `metadata` states.
    ///
    /// ```rust
    /// use waterui_core::animation::{Animation, ChangeAnimation, NoAnimation};
    /// use nami::watcher::Metadata;
    ///
    /// assert_eq!(ChangeAnimation::of(&Metadata::new()), ChangeAnimation::Unspecified);
    /// assert_eq!(
    ///     ChangeAnimation::of(&Metadata::new().with(Animation::Default)),
    ///     ChangeAnimation::Animated(Animation::Default),
    /// );
    /// assert_eq!(
    ///     ChangeAnimation::of(&Metadata::new().with(Animation::Default).with(NoAnimation)),
    ///     ChangeAnimation::Disabled,
    /// );
    /// ```
    #[must_use]
    pub fn of(metadata: &Metadata) -> Self {
        if metadata.try_get::<NoAnimation>().is_some() {
            return Self::Disabled;
        }
        metadata
            .try_get::<Animation>()
            .map_or(Self::Unspecified, Self::Animated)
    }

    /// The animation the change names, or `None` when it names none or
    /// disables animation — for a receiver whose own default is to snap.
    #[must_use]
    pub const fn animation(self) -> Option<Animation> {
        match self {
            Self::Animated(animation) => Some(animation),
            Self::Unspecified | Self::Disabled => None,
        }
    }

    /// The animation the change runs with on a receiver whose default is
    /// `default`: the named animation, `default` when the change names none,
    /// and `None` when it disables animation.
    #[must_use]
    pub const fn or(self, default: Animation) -> Option<Animation> {
        match self {
            Self::Animated(animation) => Some(animation),
            Self::Unspecified => Some(default),
            Self::Disabled => None,
        }
    }
}

/// Extension trait providing animation methods for reactive values.
///
/// This is the only animation extension trait in the framework: `waterui`
/// re-exports it from its prelude, so `.animated()` means the same thing
/// everywhere regardless of which import brought the trait into scope.
pub trait AnimationExt: nami::SignalExt {
    /// Attach the system-default animation to this reactive value.
    ///
    /// Equivalent to `self.with(Animation::Default)`; the backend picks the
    /// curve and duration that match the platform. To pick an explicit
    /// animation, use [`nami::SignalExt::with`] directly:
    /// `value.with(Animation::spring(100.0, 10.0))`.
    #[track_caller]
    fn animated(&self) -> WithMetadata<Self, Animation> {
        self.with(Animation::Default)
    }
}

// Implement AnimationExt for all types that implement SignalExt
impl<S: nami::SignalExt> AnimationExt for S {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::easing::SETTLE_TOLERANCE;

    #[test]
    fn convenience_curves_use_bezier_variant() {
        assert!(matches!(
            Animation::linear(Duration::from_millis(100)),
            Animation::Bezier {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
                ..
            }
        ));
        assert!(matches!(
            Animation::ease_in(Duration::from_millis(100)),
            Animation::Bezier {
                x1: 0.42,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
                ..
            }
        ));
        assert!(matches!(
            Animation::ease_out(Duration::from_millis(100)),
            Animation::Bezier {
                x1: 0.0,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
                ..
            }
        ));
        assert!(matches!(
            Animation::ease_in_out(Duration::from_millis(100)),
            Animation::Bezier {
                x1: 0.42,
                y1: 0.0,
                x2: 0.58,
                y2: 1.0,
                ..
            }
        ));
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "finite stiffness > 0")]
    fn spring_constructor_asserts_non_positive_stiffness_in_debug() {
        let _ = Animation::spring(0.0, 10.0);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "x1/x2 in [0, 1]")]
    fn bezier_constructor_asserts_invalid_x_range_in_debug() {
        let _ = Animation::bezier(Duration::from_millis(100), -0.1, 0.0, 0.5, 1.0);
    }

    #[test]
    fn validated_keeps_valid_animations() {
        let spring = Animation::spring(100.0, 10.0);
        assert_eq!(spring.clone().validated(), spring);
        let bezier = Animation::ease_in_out(Duration::from_millis(100));
        assert_eq!(bezier.clone().validated(), bezier);
        assert_eq!(Animation::Default.validated(), Animation::Default);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "invalid animation parameters")]
    fn validated_reports_an_invalid_spring_in_debug() {
        let _ = Animation::Spring {
            stiffness: 100.0,
            damping: -1.0,
        }
        .validated();
    }

    #[test]
    #[cfg(not(debug_assertions))]
    fn validated_replaces_invalid_animations_with_the_default_in_release() {
        let spring = Animation::Spring {
            stiffness: 0.0,
            damping: 10.0,
        };
        assert_eq!(spring.validated(), Animation::Default);
        let bezier = Animation::Bezier {
            duration: Duration::from_millis(100),
            x1: -0.1,
            y1: 0.0,
            x2: 0.5,
            y2: 1.0,
        };
        assert_eq!(bezier.validated(), Animation::Default);
    }

    #[test]
    fn animation_track_advances_to_target() {
        let mut track = AnimationTrack::new(0.0_f32);
        track.set_target(
            1.0,
            Some(Animation::ease_in_out(Duration::from_millis(120))),
        );
        assert!(track.advance(Duration::from_millis(60)));
        let mid = track.value();
        assert!(mid > 0.0 && mid < 1.0);
        assert!(!track.advance(Duration::from_millis(120)));
        assert!((track.value() - 1.0).abs() < 0.0001);
    }

    /// The step response of `Animation::spring(100, 10)`: natural frequency
    /// 10 rad/s, damping ratio 0.5, damped frequency 5√3 rad/s.
    fn spring_step_response(seconds: f64) -> f64 {
        let damped = 5.0 * 3.0_f64.sqrt();
        let decay = (-5.0 * seconds).exp();
        (decay * (5.0 / damped)).mul_add(
            -(damped * seconds).sin(),
            decay.mul_add(-(damped * seconds).cos(), 1.0),
        )
    }

    #[test]
    fn a_spring_runs_in_real_seconds() {
        let spring = Animation::spring(100.0, 10.0);
        for millis in [50_u64, 200, 363, 600, 900] {
            let progress = spring.progress(Duration::from_millis(millis));
            let expected = spring_step_response(Duration::from_millis(millis).as_secs_f64());
            assert!(
                (progress - expected).abs() < 1e-12,
                "{millis} ms: {progress} vs {expected}"
            );
        }
        // The first peak, π/ω_d ≈ 363 ms in, overshoots by e^(−πζ/√(1−ζ²)).
        let peak = spring.progress(Duration::from_millis(363));
        assert!((peak - 1.163).abs() < 1e-3, "peak {peak}");
        // Nothing snaps at 600 ms: the spring is still moving.
        assert!(!spring.is_complete(Duration::from_millis(600)));
        let at_600 = spring.progress(Duration::from_millis(600));
        assert!((at_600 - 1.0).abs() > SETTLE_TOLERANCE, "600 ms: {at_600}");
        // It settles, exactly, by its duration.
        let duration = spring.duration();
        assert!(duration > Duration::from_secs(1) && duration < Duration::from_secs(2));
        assert!(spring.is_complete(duration));
        assert_eq!(spring.progress(duration).to_bits(), 1.0_f64.to_bits());
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "finite damping > 0")]
    fn an_undamped_spring_is_rejected_in_debug() {
        let _ = Animation::spring(100.0, 0.0);
    }

    #[test]
    fn a_retargeted_spring_carries_its_velocity() {
        let animation = Animation::spring(100.0, 10.0);
        let timing = animation.timing();
        let mut track = AnimationTrack::new(0.0_f64);
        track.set_target(1.0, Some(animation.clone()));
        assert!(track.advance(Duration::from_millis(100)));
        // The state the interrupted spring had reached, from the evaluator.
        let (mut position, mut velocity) = ([0.0], [0.0]);
        timing.sample_lanes(&[0.0], &[0.0], &[1.0], 0.1, &mut position, &mut velocity);
        assert_eq!(track.value().to_bits(), position[0].to_bits());
        assert!(velocity[0] > 1.0, "the spring is moving fast: {velocity:?}");

        track.set_target(-1.0, Some(animation));
        assert!(track.advance(Duration::from_millis(30)));
        let (mut expected, mut rate) = ([0.0], [0.0]);
        timing.sample_lanes(
            &position,
            &velocity,
            &[-1.0],
            0.03,
            &mut expected,
            &mut rate,
        );
        assert_eq!(track.value().to_bits(), expected[0].to_bits());
        // Thirty milliseconds after turning back, it is still carried past
        // where it was by the velocity it had.
        assert!(
            track.value() > position[0],
            "{} vs {}",
            track.value(),
            position[0]
        );
    }

    #[test]
    fn a_spring_track_ends_exactly_on_its_target() {
        let mut track = AnimationTrack::new([0.0_f32, 40.0]);
        track.set_target([120.0, -8.0], Some(Animation::spring(300.0, 15.0)));
        let mut frames = 0;
        while track.advance(Duration::from_millis(8)) {
            frames += 1;
            assert!(frames < 1000, "the spring settles within eight seconds");
        }
        assert_eq!(track.value(), [120.0, -8.0]);
        assert!(!track.is_active());
    }

    #[test]
    fn change_animation_reads_the_three_cases() {
        let spring = Animation::spring(100.0, 10.0);
        assert_eq!(
            ChangeAnimation::of(&Metadata::new()),
            ChangeAnimation::Unspecified
        );
        assert_eq!(
            ChangeAnimation::of(&Metadata::new().with(spring.clone())),
            ChangeAnimation::Animated(spring)
        );
        assert_eq!(
            ChangeAnimation::of(&Metadata::new().with(NoAnimation)),
            ChangeAnimation::Disabled
        );
    }

    #[test]
    fn no_animation_wins_over_an_attached_animation() {
        let inner = Metadata::new().with(NoAnimation).with(Animation::Default);
        let outer = Metadata::new().with(Animation::Default).with(NoAnimation);
        assert_eq!(ChangeAnimation::of(&inner), ChangeAnimation::Disabled);
        assert_eq!(ChangeAnimation::of(&outer), ChangeAnimation::Disabled);
    }

    #[test]
    fn change_animation_resolves_against_a_receiver_default() {
        let default = Animation::linear(Duration::from_millis(80));
        let named = Animation::ease_in(Duration::from_millis(40));
        assert_eq!(
            ChangeAnimation::Animated(named.clone()).or(default.clone()),
            Some(named.clone())
        );
        assert_eq!(
            ChangeAnimation::Unspecified.or(default.clone()),
            Some(default)
        );
        assert_eq!(ChangeAnimation::Disabled.or(named.clone()), None);
        assert_eq!(
            ChangeAnimation::Animated(named.clone()).animation(),
            Some(named)
        );
        assert_eq!(ChangeAnimation::Unspecified.animation(), None);
        assert_eq!(ChangeAnimation::Disabled.animation(), None);
    }

    #[test]
    fn signal_with_no_animation_reaches_watchers_as_disabled() {
        use alloc::rc::Rc;
        use core::cell::RefCell;
        use nami::{Signal, SignalExt, binding};

        let value: nami::Binding<f32> = binding(0.0_f32);
        let seen = Rc::new(RefCell::new(None));
        let sink = Rc::clone(&seen);
        let snapping = value.animated().with(NoAnimation);
        let _guard = snapping.watch(move |context| {
            *sink.borrow_mut() = Some(ChangeAnimation::of(context.metadata()));
        });
        value.set(1.0);
        assert_eq!(seen.borrow().clone(), Some(ChangeAnimation::Disabled));
    }

    #[derive(Clone)]
    struct Pair {
        x: f32,
        y: f32,
    }

    impl Animatable for Pair {
        type AnimatableData = (f32, f32);

        fn animatable_data(&self) -> Self::AnimatableData {
            (self.x, self.y)
        }

        fn from_animatable_data(data: Self::AnimatableData) -> Self {
            Self {
                x: data.0,
                y: data.1,
            }
        }
    }

    #[test]
    fn custom_animatable_interpolates() {
        let animation = Animation::linear(Duration::from_millis(100));
        let from = Pair { x: 0.0, y: 0.0 };
        let to = Pair { x: 10.0, y: 20.0 };
        let value = animation.interpolate(&from, &to, Duration::from_millis(50));
        assert!((value.x - 5.0).abs() < 0.001);
        assert!((value.y - 10.0).abs() < 0.001);
    }
}
