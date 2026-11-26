//! # Focus Management System
//!
//! This module provides a reactive focus management system for `WaterUI` applications.
//! It allows tracking which UI element currently has focus, enabling keyboard navigation
//! and accessibility features.
//!
//! # Example
//!
//! ```
//! use waterui::prelude::*;
//! use waterui::ViewExt;
//! use waterui_core::{Str, binding};
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
//!     TextField::new("Username", &binding(Str::from(""))).focused(&focus_binding, Field::Username),
//!     SecureField::new("Password", &binding(Secure::default()))
//!         .focused(&focus_binding, Field::Password),
//! ));
//! ```

pub use waterui_interaction::focus::Focused;
