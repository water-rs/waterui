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
//! The system supports various animation types:
//!
//! - **`Linear`**: Constant velocity from start to finish
//! - **`EaseIn`**: Starts slow and accelerates
//! - **`EaseOut`**: Starts fast and decelerates
//! - **`EaseInOut`**: Combines ease-in and ease-out for natural movement
//! - **`Spring`**: Physics-based animation with configurable stiffness and damping
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
//!     .zip(value2)
//!     .map(|(a, b)| a + b)
//!     .with_animation(Animation::ease_in_out(Duration::from_millis(250)));
//!
//! drop((opacity, combined)); // Prevent unused variable warnings
//! ```
//!

use core::time::Duration;

use crate::easing::{EasingCurve, Interpolatable};

/// Default spring animation duration (used for timing calculations).
const DEFAULT_SPRING_DURATION: Duration = Duration::from_millis(600);

/// An enumeration representing different types of animations
///
/// This enum provides various animation types for UI elements or graphics:
/// - `Linear`: Constant speed from start to finish
/// - `EaseIn`: Starts slow and accelerates
/// - `EaseOut`: Starts fast and decelerates
/// - `EaseInOut`: Starts and ends slowly with acceleration in the middle
/// - `CubicBezier`: Custom bezier curve with control points
/// - `Spring`: Physics-based movement with configurable stiffness and damping
///
/// Each animation type (except Spring) takes a Duration parameter that specifies
/// how long the animation should take to complete.
#[derive(Debug, Default, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Animation {
    /// Default animation behavior (uses system defaults)
    #[default]
    Default,
    /// Linear animation with constant velocity
    Linear(Duration),
    /// Ease-in animation that starts slow and accelerates
    EaseIn(Duration),
    /// Ease-out animation that starts fast and decelerates
    EaseOut(Duration),
    /// Ease-in-out animation that starts and ends slowly with acceleration in the middle
    EaseInOut(Duration),
    /// Custom cubic bezier animation with control points (x1, y1, x2, y2)
    CubicBezier {
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
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
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
    pub fn linear(duration: impl Into<Duration>) -> Self {
        Self::Linear(duration.into())
    }

    /// Creates a new ease-in animation with the specified duration
    ///
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
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
    pub fn ease_in(duration: impl Into<Duration>) -> Self {
        Self::EaseIn(duration.into())
    }

    /// Creates a new ease-out animation with the specified duration
    ///
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
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
    pub fn ease_out(duration: impl Into<Duration>) -> Self {
        Self::EaseOut(duration.into())
    }

    /// Creates a new ease-in-out animation with the specified duration
    ///
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
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
    pub fn ease_in_out(duration: impl Into<Duration>) -> Self {
        Self::EaseInOut(duration.into())
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
    pub fn bezier(duration: impl Into<Duration>, x1: f32, y1: f32, x2: f32, y2: f32) -> Self {
        Self::CubicBezier {
            duration: duration.into(),
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
            Self::Linear(_) => EasingCurve::LINEAR,
            Self::EaseIn(_) => EasingCurve::EASE_IN,
            Self::EaseOut(_) => EasingCurve::EASE_OUT,
            Self::EaseInOut(_) => EasingCurve::EASE_IN_OUT,
            Self::CubicBezier { x1, y1, x2, y2, .. } => EasingCurve::bezier(*x1, *y1, *x2, *y2),
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
            Self::Default => Duration::from_millis(250),
            Self::Linear(d)
            | Self::EaseIn(d)
            | Self::EaseOut(d)
            | Self::EaseInOut(d)
            | Self::CubicBezier { duration: d, .. } => *d,
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
