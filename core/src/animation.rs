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
//! use waterui_core::{color::Color, animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let color: nami::Binding<Color> = binding(Color::from((255, 0, 0))); // Red color
//!
//! // Use the .animated() method to apply default animation
//! let _animated_color = color.animated();
//!
//! // Or specify a specific animation type
//! let color2: nami::Binding<Color> = binding(Color::from((0, 255, 0)));
//! let _custom_animated = color2.with_animation(Animation::ease_in_out(Duration::from_millis(300)));
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
//! use waterui_core::{color::Color, animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let color: nami::Binding<Color> = binding(Color::from((255, 0, 0)));
//!
//! // Three different ways to animate properties:
//!
//! // 1. Default animation (uses system defaults)
//! let _view1_color = color.animated();
//!
//! // 2. Custom animation using convenience methods
//! let color2: nami::Binding<Color> = binding(Color::from((0, 255, 0)));
//! let _view2_color = color2.with_animation(Animation::ease_in_out(Duration::from_millis(300)));
//!
//! // 3. Spring animation using the convenience method
//! let color3: nami::Binding<Color> = binding(Color::from((0, 0, 255)));
//! let _view3_color = color3.with_animation(Animation::spring(100.0, 10.0));
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
//! use waterui_core::{color::Color, animation::Animation, AnimationExt, SignalExt};
//! use nami::binding;
//! use core::time::Duration;
//!
//! let color: nami::Binding<Color> = binding(Color::from((255, 0, 0)));
//! let position: nami::Binding<(i32, i32)> = binding((0, 0));
//!
//! // Create a choreographed animation sequence
//! let animated_color = color.with_animation(Animation::ease_in_out(Duration::from_millis(300)));
//!
//! // Position animates with a spring physics model
//! let animated_position = position.with_animation(Animation::spring(100.0, 10.0));
//!
//! // Both animated values can be used in views
//! // The UI framework will automatically handle the animation timing
//! drop((animated_color, animated_position)); // Prevent unused variable warnings
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

/// An enumeration representing different types of animations
///
/// This enum provides various animation types for UI elements or graphics:
/// - `Linear`: Constant speed from start to finish
/// - `EaseIn`: Starts slow and accelerates
/// - `EaseOut`: Starts fast and decelerates
/// - `EaseInOut`: Starts and ends slowly with acceleration in the middle
/// - `Spring`: Physics-based movement with configurable stiffness and damping
///
/// Each animation type (except Spring) takes a Duration parameter that specifies
/// how long the animation should take to complete.
#[derive(Debug, Default, Clone, PartialEq)]

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
