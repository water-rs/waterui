//! # Focus Management System
//!
//! This module provides a reactive focus management system for `WaterUI` applications.
//! It allows tracking which UI element currently has focus, enabling keyboard navigation
//! and accessibility features.
//!
//! ## Focus Model
//!
//! The focus system operates on the principle that only one element can have focus at a time.
//! Focus state is tracked through a shared reactive binding, allowing different parts of the
//! application to observe and modify the currently focused element.
//!
//! ```text
//! ┌───────────────────────────────────────────┐
//! │                Application                │
//! │                                           │
//! │  ┌─────────────┐      ┌─────────────┐     │
//! │  │   Element   │      │   Element   │     │
//! │  │ (unfocused) │      │  (focused)  │     │
//! │  └─────────────┘      └─────────────┘     │
//! │                                           │
//! │  ┌─────────────┐      ┌─────────────┐     │
//! │  │   Element   │      │   Element   │     │
//! │  │ (unfocused) │      │ (unfocused) │     │
//! │  └─────────────┘      └─────────────┘     │
//! │                                           │
//! └───────────────────────────────────────────┘
//! ```
//!
//! # Example
//!
//! ```
//! use waterui::prelude::*;
//!
//! #[derive(PartialEq, Eq, Clone)]
//! enum Field { Username, Password, Submit }
//!
//! // Create a shared binding for focus state using an enum
//! let focus_binding = binding(None::<Field>);
//!
//! // Create focused states for each field
//! let username_focused = focu::Focused::new(&focus_binding, Field::Username);
//! let password_focused = focu::Focused::new(&focus_binding, Field::Password);
//! let submit_focused = focu::Focused::new(&focus_binding, Field::Submit);
//!
//! // Use focused states with UI elements
//! let view = vstack((
//!     text("Username").focused(&focus_binding, Field::Username),
//!     text("Password").focused(&focus_binding, Field::Password),
//!     button("Submit").focused(&focus_binding, Field::Submit),
//! ));
//! ```
//!
//! When one element receives focus, any previously focused element will automatically
//! lose focus due to the shared binding mechanism.

use crate::Binding;

/// A struct that represents a focused state based on a binding to a boolean value.
#[derive(Debug, Clone)]
pub struct Focused(pub Binding<bool>);

impl Focused {
    /// Creates a new `Focused` instance based on an optional value binding.
    ///
    /// This function creates a binding that is true when the provided `value` binding
    /// contains a value that equals the provided `equals` parameter.
    ///
    /// # Parameters
    /// - `value`: A binding to an optional value.
    /// - `equals`: The value to compare against.
    ///
    /// # Returns
    /// A new `Focused` instance.
    pub fn new<T: 'static + Eq + Clone>(value: &Binding<Option<T>>, equals: T) -> Self {
        Self(Binding::mapping(
            value,
            {
                let equals = equals.clone();
                move |value| value.as_ref().filter(|value| **value == equals).is_some()
            },
            move |binding, value| {
                if value {
                    binding.set(Some(equals.clone()));
                }
            },
        ))
    }
}
