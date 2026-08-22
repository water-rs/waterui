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
//! use waterui::ViewExt;
//! use waterui_core::binding;
//! use waterui_form::{SecureField, secure::Secure};
//! use waterui_controls::TextField;
//! use waterui_layout::stack::vstack;
//!
//! #[derive(PartialEq, Eq, Clone)]
//! enum Field { Username, Password }
//!
//! // Create a shared binding for focus state using an enum
//! let focus_binding = binding(None::<Field>);
//!
//! // Create focused states for each field
//! let username_focused = focus::Focused::new(&focus_binding, Field::Username);
//! let password_focused = focus::Focused::new(&focus_binding, Field::Password);
//!
//! // Use focused states with UI elements
//! let view = vstack((
//!     TextField::new("Username", &binding("")).focused(&focus_binding, Field::Username),
//!     SecureField::new("Password", &binding(Secure::default()))
//!         .focused(&focus_binding, Field::Password),
//! ));
//! ```
//!
//! When one element receives focus, any previously focused element will automatically
//! lose focus due to the shared binding mechanism.

use waterui_core::metadata::MetadataKey;

use crate::Binding;

/// A struct that represents a focused state based on a binding to a boolean value.
#[derive(Debug, Clone)]
pub struct Focused(pub Binding<bool>);

impl MetadataKey for Focused {}

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
                move |value| value.as_ref().is_some_and(|value| value == &equals)
            },
            move |binding, is_focused| {
                if is_focused {
                    binding.set(Some(equals.clone()));
                    return;
                }

                if binding
                    .get()
                    .as_ref()
                    .is_some_and(|current| *current == equals)
                {
                    binding.set(None);
                }
            },
        ))
    }
}

#[cfg(test)]
mod tests {
    use nami::Binding;

    use super::Focused;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Field {
        Username,
        Password,
    }

    #[test]
    fn clearing_matching_focus_sets_source_to_none() {
        let source = Binding::container(Some(Field::Username));
        let focused = Focused::new(&source, Field::Username);

        focused.0.set(false);

        assert_eq!(source.get(), None);
    }

    #[test]
    fn clearing_other_field_focus_is_ignored() {
        let source = Binding::container(Some(Field::Password));
        let focused = Focused::new(&source, Field::Username);

        focused.0.set(false);

        assert_eq!(source.get(), Some(Field::Password));
    }
}
