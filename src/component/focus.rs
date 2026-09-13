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
    use std::cell::RefCell;
    use std::rc::Rc;

    use nami::{Binding, Signal};

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

    #[test]
    fn reads_true_only_while_source_holds_the_matching_value() {
        let source = Binding::container(Some(Field::Username));
        let focused = Focused::new(&source, Field::Username);

        assert!(focused.0.get());
        source.set(Some(Field::Password));
        assert!(!focused.0.get());
        source.set(None);
        assert!(!focused.0.get());
    }

    #[test]
    fn setting_focus_writes_the_matching_value() {
        let source = Binding::container(None::<Field>);
        let focused = Focused::new(&source, Field::Password);

        focused.0.set(true);

        assert_eq!(source.get(), Some(Field::Password));
    }

    #[test]
    fn focusing_one_field_unfocuses_the_other() {
        let source = Binding::container(Some(Field::Username));
        let username = Focused::new(&source, Field::Username);
        let password = Focused::new(&source, Field::Password);

        password.0.set(true);

        assert_eq!(source.get(), Some(Field::Password));
        assert!(!username.0.get());
        assert!(password.0.get());
    }

    #[test]
    fn clearing_focus_on_empty_source_is_a_no_op() {
        let source = Binding::container(None::<Field>);
        let focused = Focused::new(&source, Field::Username);

        focused.0.set(false);

        assert_eq!(source.get(), None);
    }

    #[test]
    fn source_changes_reach_focused_watchers() {
        let source = Binding::container(Some(Field::Username));
        let focused = Focused::new(&source, Field::Username);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let _guard = focused.0.watch({
            let seen = Rc::clone(&seen);
            move |ctx| seen.borrow_mut().push(ctx.into_value())
        });

        source.set(Some(Field::Password));
        source.set(None);

        assert_eq!(*seen.borrow(), vec![false, false]);
    }
}
