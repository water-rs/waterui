//! Secure form components for handling sensitive data.
//!
//! This module provides utilities for handling sensitive form data such as
//! passwords and other secrets with automatic memory zeroing for security.

use core::fmt::Debug;

use alloc::string::String;
use nami::Binding;
use waterui_core::{AnyView, View, configurable};
use zeroize::Zeroize;

/// A wrapper type for securely handling sensitive string data.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Secure(String);

impl Debug for Secure {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Secure(****)")
    }
}

impl Secure {
    /// Returns the inner string as a string slice.
    ///
    /// # Returns
    ///
    /// A reference to the inner string data.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Hashes the secure string using bcrypt.
    ///
    /// # Returns
    ///
    /// A bcrypt hash of the inner string data.
    #[allow(clippy::missing_panics_doc)] // bcrypt::hash never panics
    #[must_use]
    pub fn hash(&self) -> String {
        bcrypt::hash(self.expose(), bcrypt::DEFAULT_COST).expect("Failed to hash password")
    }
}

// Ensure the inner string is zeroed out when dropped
impl Drop for Secure {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Configuration for a secure field component.
#[derive(Debug)]
pub struct SecureFieldConfig {
    /// The label view displayed for the secure field.
    pub label: AnyView,
    /// The binding to the secure value being edited.
    pub value: Binding<Secure>,
}

configurable!(
    #[doc = "A secure text entry field that stores its value in a zeroizing wrapper."]
    SecureField,
    SecureFieldConfig
);

impl SecureField {
    /// Creates a new `SecureField` instance.
    ///
    /// # Arguments
    ///
    /// * `label` - A view representing the label for the secure field.
    /// * `value` - A binding to the `Secure` value that the field will edit.
    ///
    /// # Returns
    ///
    /// A new `SecureField` instance configured with the provided label and value binding.
    #[must_use]
    pub fn new(label: impl View, value: &Binding<Secure>) -> Self {
        Self(SecureFieldConfig {
            label: AnyView::new(label),
            value: value.clone(),
        })
    }

    /// Sets the label for the secure field.
    ///
    /// # Arguments
    ///
    /// * `label` - A view representing the new label for the secure field.
    ///
    /// # Returns
    ///
    /// A new `SecureField` instance with the updated label.
    #[must_use]
    pub fn label(self, label: impl View) -> Self {
        let mut config = self.0;
        config.label = AnyView::new(label);
        Self(config)
    }
}

/// Creates a new `SecureField` instance.
/// See [`SecureField::new`] for more details.
#[must_use]
pub fn secure(label: impl View, value: &Binding<Secure>) -> SecureField {
    SecureField::new(label, value)
}
