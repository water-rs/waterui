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
//! ## Usage Example
//!
//! ```rust,no_run
//! use waterui::{Binding, Focused};
//!
//! // Create a shared binding for focus state using an enum
//! enum Field { Username, Password, Submit }
//! let focus_binding = Binding::new(None::<Field>);
//!
//! // Create focused states for each element
//! let username_focused = Focused::new(focus_binding.clone(), Field::Username);
//! let password_focused = Focused::new(focus_binding.clone(), Field::Password);
//! let submit_focused = Focused::new(focus_binding.clone(), Field::Submit);
//!
//! // Use focused states with UI elements
//! // TextInput::new().focused(username_focused)
//! // TextInput::new().focused(password_focused)
//! // Button::new().focused(submit_focused)
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
