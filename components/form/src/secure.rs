//! Secure form components for handling sensitive data.
//!
//! This module provides utilities for handling sensitive form data such as
//! passwords and other secrets with automatic memory zeroing for security.

use core::fmt::Debug;

use alloc::string::{String, ToString};
use nami::Binding;
use waterui_core::{AnyView, View, configurable, layout::StretchAxis};
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
    /// Creates a new Secure value from a string.
    ///
    /// # Arguments
    ///
    /// * `value` - The string value to secure
    ///
    /// # Returns
    ///
    /// A new Secure instance wrapping the provided string.
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// Creates a new Secure value from a string slice.
    ///
    /// # Arguments
    ///
    /// * `value` - The string slice to secure
    ///
    /// # Returns
    ///
    /// A new Secure instance wrapping a copy of the provided string.
    #[must_use]
    pub fn from_str(value: &str) -> Self {
        Self(value.to_string())
    }

    /// Returns the inner string as a string slice.
    ///
    /// # Returns
    ///
    /// A reference to the inner string data.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Sets the value of the secure string.
    ///
    /// # Arguments
    ///
    /// * `value` - The new string value
    pub fn set(&mut self, value: String) {
        self.0.zeroize();
        self.0 = value;
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
    /// A secure text entry field for passwords and sensitive data.
    ///
    /// SecureField masks input and securely stores values with automatic memory zeroing.
    ///
    /// # Layout Behavior
    ///
    /// SecureField **expands horizontally** to fill available space, but has a fixed height.
    /// In an `HStack`, it will take up all remaining width after other views are sized.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    // INTERNAL: Layout Contract for Backend Implementers
    // ═══════════════════════════════════════════════════════════════════════════
    //

    // Height: Fixed intrinsic (platform-determined)
    // Width: Reports minimum usable width, expands during layout phase
    //
    // Same layout behavior as TextField.
    //
    // ═══════════════════════════════════════════════════════════════════════════
    //
    SecureField,
    SecureFieldConfig,
    StretchAxis::Horizontal
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
