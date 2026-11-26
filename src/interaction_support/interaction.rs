//! Interaction modifiers for controlling view interactivity.
//!
//! This module provides modifiers for controlling how views respond to user interactions
//! such as touch events and hit testing.
//!
//! # Example
//!
//! ```rust
//! use waterui::prelude::*;
//!
//! // Make a view transparent to touch events
//! let passthrough = text!("Click through me")
//!     .hittable(false);
//!
//! // Reactive hit testing control
//! let can_interact = binding::<bool>(true);
//! button("Click me").action(|| {})
//!     .hittable(can_interact);
//! ```

pub use waterui_interaction::interaction::{
    Disabled, Hittable, InteractionReport, InteractionState, Selected, StateValue,
};
