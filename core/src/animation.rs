//! # WaterUI Animation System
//!
//! A reactive animation system that seamlessly integrates with WaterUI's reactive state management.
//!
//! ## Overview
//!
//! The WaterUI animation system leverages the reactive framework to create smooth, declarative
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
//! WaterUI provides convenient extension methods on all `Compute` types to easily attach
//! animation configurations:
//!
//! ```rust
//! let color = binding(Color::RED);
//!
//! // Use the .animated() method to apply default animation
//! view.background(color.animated());
//!
//! // Or specify a specific animation type
//! view.background(color.with(Animation::ease_in_out(300)));
//! ```
//!
//! ### Animation Types
//!
//! The system supports various animation types:
//!
//! - **Linear**: Constant velocity from start to finish
//! - **EaseIn**: Starts slow and accelerates
//! - **EaseOut**: Starts fast and decelerates
//! - **EaseInOut**: Combines ease-in and ease-out for natural movement
//! - **Spring**: Physics-based animation with configurable stiffness and damping
//!
//! ### Integration with UI Components
//!
//! UI components automatically respect animation metadata when rendering:
//!
//! ```rust
//! // Three different ways to animate properties:
//!
//! // 1. Default animation (uses system defaults)
//! let view = View::new()
//!     .background(color.animated());
//!
//! // 2. Custom animation using convenience methods
//! let view = View::new()
//!     .background(color.with(Animation::ease_in_out(300)));
//!
//! // 3. Spring animation using the convenience method
//! let view = View::new()
//!     .background(color.with(Animation::spring(100.0, 10.0)));
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
//! // Create a choreographed animation sequence
//! let animated_color = color.with(Animation::ease_in_out(300));
//!
//! // Position animates with a spring physics model
//! let animated_position = position.with(
//!     Animation::spring(100.0, 10.0)
//! );
//!
//! // Use both animated values in your view
//! let view = View::new()
//!     .background(animated_color)
//!     .position(animated_position);
//! ```
//!
//! ### Composition with Other Reactive Features
//!
//! Animation metadata seamlessly composes with other reactive features:
//!
//! ```rust
//! // Combine mapping and animation
//! let opacity = count
//!     .map(|n| if n > 5 { 1.0 } else { 0.5 })
//!     .animated();  // Apply animation to the mapped result
//!
//! // Combine multiple reactive values with animation
//! let combined = value1
//!     .zip(value2)
//!     .map(|(a, b)| a + b)
//!     .with(Animation::ease_in_out(250));
//! ```
//!

use core::time::Duration;

/// An enumeration representing different types of animations
///
/// This enum provides various animation types for UI elements or graphics:
/// - Linear: Constant speed from start to finish
/// - EaseIn: Starts slow and accelerates
/// - EaseOut: Starts fast and decelerates
/// - EaseInOut: Starts and ends slowly with acceleration in the middle
/// - Spring: Physics-based movement with configurable stiffness and damping
///
/// Each animation type (except Spring) takes a Duration parameter that specifies
/// how long the animation should take to complete.
#[derive(Debug, Default, Clone, PartialEq)]
pub enum Animation {
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
    /// let animation = Animation::linear(300); // 300ms
    /// let animation = Animation::linear(Duration::from_secs(1)); // 1 second
    /// ```
    pub fn linear(duration: impl Into<Duration>) -> Self {
        Animation::Linear(duration.into())
    }

    /// Creates a new EaseIn animation with the specified duration
    ///
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
    ///
    /// # Examples
    ///
    /// ```
    /// let animation = Animation::ease_in(300); // 300ms
    /// let animation = Animation::ease_in(Duration::from_secs(1)); // 1 second
    /// ```
    pub fn ease_in(duration: impl Into<Duration>) -> Self {
        Animation::EaseIn(duration.into())
    }

    /// Creates a new EaseOut animation with the specified duration
    ///
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
    ///
    /// # Examples
    ///
    /// ```
    /// let animation = Animation::ease_out(300); // 300ms
    /// let animation = Animation::ease_out(Duration::from_secs(1)); // 1 second
    /// ```
    pub fn ease_out(duration: impl Into<Duration>) -> Self {
        Animation::EaseOut(duration.into())
    }

    /// Creates a new EaseInOut animation with the specified duration
    ///
    /// This is an ergonomic constructor that accepts any type that can be converted
    /// into a Duration (such as u64 milliseconds, etc.)
    ///
    /// # Examples
    ///
    /// ```
    /// let animation = Animation::ease_in_out(300); // 300ms
    /// let animation = Animation::ease_in_out(Duration::from_secs(1)); // 1 second
    /// ```
    pub fn ease_in_out(duration: impl Into<Duration>) -> Self {
        Animation::EaseInOut(duration.into())
    }

    /// Creates a new Spring animation with the specified stiffness and damping
    ///
    /// # Examples
    ///
    /// ```
    /// let animation = Animation::spring(100.0, 10.0);
    /// ```
    pub fn spring(stiffness: f32, damping: f32) -> Self {
        Animation::Spring { stiffness, damping }
    }
}
