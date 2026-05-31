//! Secure form components for handling sensitive data.
//!
//! This module provides utilities for handling sensitive form data such as
//! passwords and other secrets with automatic memory zeroing for security.

use core::fmt;
use core::{fmt::Debug, str::FromStr};

use alloc::string::{String, ToString};
use nami::Binding;
use waterui_controls::label::Label;
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::{Environment, configurable, layout::StretchAxis};
use zeroize::Zeroize;

/// A wrapper type for securely handling sensitive string data.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Secure(String);

impl Debug for Secure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secure(****)")
    }
}

impl FromStr for Secure {
    type Err = core::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(s.to_string()))
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
    pub const fn new(value: String) -> Self {
        Self(value)
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
    /// The label displayed for the secure field.
    pub label: Label,
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
    StretchAxis::Horizontal,
    resolve |config, env| config.resolve(env)
);

impl SecureFieldConfig {
    #[must_use]
    fn resolve(mut self, env: &Environment) -> Self {
        self.label = self.label.resolve(env);
        self
    }
}

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
    pub fn new(label: impl IntoLabel, value: &Binding<Secure>) -> Self {
        Self(SecureFieldConfig {
            label: label.into_label(),
            value: value.clone(),
        })
    }
}

impl_label_style_methods!(SecureField);

/// Creates a new `SecureField` instance.
/// See [`SecureField::new`] for more details.
#[must_use]
pub fn secure(label: impl IntoLabel, value: &Binding<Secure>) -> SecureField {
    SecureField::new(label, value)
}
