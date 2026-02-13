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
//! // Use the .animated() method to apply default animation
//! let _animated_opacity = opacity.clone().animated();
//!
//! // Or specify a specific animation type
//! let faded: nami::Binding<f32> = binding(0.0);
//! let _custom_animated =
//!     faded.with_animation(Animation::ease_in_out(Duration::from_millis(300)));
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
//! let _view1_scale = scale.clone().animated();
//!
//! // 2. Custom animation using convenience methods
//! let expanded: nami::Binding<f32> = binding(2.0);
//! let _view2_scale =
//!     expanded.with_animation(Animation::ease_in_out(Duration::from_millis(300)));
//!
//! // 3. Spring animation using the convenience method
//! let bouncing: nami::Binding<f32> = binding(0.5);
//! let _view3_scale = bouncing.with_animation(Animation::spring(100.0, 10.0));
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
//! use waterui_core::{animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let opacity: nami::Binding<f32> = binding(0.0);
//! let position: nami::Binding<(f32, f32)> = binding((0.0, 0.0));
//!
//! // Create a choreographed animation sequence
//! let animated_opacity =
//!     opacity.with_animation(Animation::ease_in_out(Duration::from_millis(300)));
//!
//! // Position animates with a spring physics model
//! let animated_position = position.with_animation(Animation::spring(100.0, 10.0));
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
//!     .with_animation(Animation::ease_in_out(Duration::from_millis(250)));
//!
//! drop((opacity, combined)); // Prevent unused variable warnings
//! ```
//!

use core::time::Duration;

use crate::easing::{EasingCurve, Interpolatable};

/// Default duration for timed animations.
const DEFAULT_TIMED_DURATION: Duration = Duration::from_millis(250);
/// Default spring animation duration (used for timing calculations).
const DEFAULT_SPRING_DURATION: Duration = Duration::from_millis(600);

/// An enumeration representing different types of animations
///
/// This enum exposes two native animation primitives:
/// - `Bezier`: Timed animations represented by cubic bezier control points
/// - `Spring`: Physics-based movement with configurable stiffness and damping
///
/// Convenience constructors (`linear`, `ease_in`, etc.) all map to `Bezier`.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
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
    /// Spring animation with physics-based movement
    Spring {
        /// Stiffness of the spring (higher values create faster animations)
        stiffness: f32,
        /// Damping factor to control oscillation (higher values reduce bouncing)
        damping: f32,
    },
}

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
    pub const fn ease_in_out(duration: Duration) -> Self {
        Self::Bezier {
            duration,
            x1: 0.42,
            y1: 0.0,
            x2: 0.58,
            y2: 1.0,
        }
    }

    /// Creates a new Spring animation with the specified stiffness and damping
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_core::animation::Animation;
    ///
    /// let animation = Animation::spring(100.0, 10.0);
    /// ```
    #[must_use]
    pub const fn spring(stiffness: f32, damping: f32) -> Self {
        if !stiffness.is_finite() || stiffness <= 0.0 {
            panic!("Animation::spring requires finite stiffness > 0");
        }
        if !damping.is_finite() || damping < 0.0 {
            panic!("Animation::spring requires finite damping >= 0");
        }
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
    pub const fn bezier(duration: Duration, x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        if !x1.is_finite() || !y1.is_finite() || !x2.is_finite() || !y2.is_finite() {
            panic!("Animation::bezier requires finite control points");
        }
        if x1 < 0.0 || x1 > 1.0 || x2 < 0.0 || x2 > 1.0 {
            panic!("Animation::bezier requires x1/x2 in [0, 1]");
        }
        Self::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }
    }

    /// Get the underlying easing curve for this animation.
    ///
    /// This allows using the unified easing system for interpolation.
    #[must_use]
    pub fn curve(&self) -> EasingCurve {
        match self {
            Self::Default => EasingCurve::EASE_IN_OUT,
            Self::Bezier { x1, y1, x2, y2, .. } => EasingCurve::bezier(*x1, *y1, *x2, *y2),
            Self::Spring { stiffness, damping } => EasingCurve::spring(*stiffness, *damping),
        }
    }

    /// Get the total duration of this animation.
    ///
    /// For spring animations, returns a default duration (600ms) since spring
    /// duration depends on the physics parameters.
    #[must_use]
    pub fn duration(&self) -> Duration {
        match self {
            Self::Default => DEFAULT_TIMED_DURATION,
            Self::Bezier { duration: d, .. } => *d,
            Self::Spring { .. } => DEFAULT_SPRING_DURATION,
        }
    }

    /// Get the eased progress for the given elapsed time.
    ///
    /// Returns a value typically between 0.0 and 1.0, though spring animations
    /// may temporarily overshoot (return values > 1.0 or < 0.0).
    ///
    /// # Arguments
    ///
    /// * `elapsed` - Time elapsed since animation started
    #[must_use]
    pub fn progress(&self, elapsed: Duration) -> f32 {
        let duration = self.duration();
        if duration.is_zero() {
            return 1.0;
        }

        let t = (elapsed.as_secs_f32() / duration.as_secs_f32()).clamp(0.0, 1.0);
        self.curve().ease(t)
    }

    /// Interpolate between two values based on elapsed time.
    ///
    /// Uses the animation's easing curve to calculate the current value.
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
    /// let value = anim.interpolate(0.0_f32, 100.0_f32, elapsed);
    /// // value is approximately 50.0, but eased
    /// ```
    pub fn interpolate<T: Interpolatable>(&self, from: T, to: T, elapsed: Duration) -> T {
        let progress = self.progress(elapsed);
        from.lerp(&to, progress)
    }

    /// Returns true if the animation is complete.
    ///
    /// An animation is complete when the elapsed time equals or exceeds its duration.
    #[must_use]
    pub fn is_complete(&self, elapsed: Duration) -> bool {
        elapsed >= self.duration()
    }
}

use nami::signal::WithMetadata;

/// Extension trait providing animation methods for reactive values
pub trait AnimationExt: nami::SignalExt {
    /// Apply default animation to this reactive value
    ///
    /// Uses a reasonable default animation (ease-in-out with 250ms duration)
    fn animated(self) -> WithMetadata<Self, Animation>
    where
        Self: Sized,
    {
        self.with(Animation::ease_in_out(Duration::from_millis(250)))
    }

    /// Apply a specific animation to this reactive value
    ///
    /// # Arguments
    ///
    /// * `animation` - The animation to apply
    fn with_animation(self, animation: Animation) -> WithMetadata<Self, Animation>
    where
        Self: Sized,
    {
        self.with(animation)
    }
}

// Implement AnimationExt for all types that implement SignalExt
impl<S: nami::SignalExt> AnimationExt for S {}

#[cfg(test)]
mod tests {
    use super::*;

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
    #[should_panic(expected = "stiffness > 0")]
    fn spring_rejects_non_positive_stiffness() {
        let _ = Animation::spring(0.0, 10.0);
    }

    #[test]
    #[should_panic(expected = "damping >= 0")]
    fn spring_rejects_negative_damping() {
        let _ = Animation::spring(100.0, -1.0);
    }

    #[test]
    #[should_panic(expected = "x1/x2 in [0, 1]")]
    fn bezier_rejects_invalid_x_range() {
        let _ = Animation::bezier(Duration::from_millis(100), -0.1, 0.0, 0.5, 1.0);
    }
}
