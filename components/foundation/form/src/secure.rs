//! Secure form components for handling sensitive data.
//!
//! This module provides utilities for handling sensitive form data such as
//! passwords and other secrets with automatic memory zeroing for security.

use core::fmt;
use core::{fmt::Debug, str::FromStr};

use alloc::string::{String, ToString};
use nami::Binding;
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};
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
    #[cfg(feature = "password-hash")]
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
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SecureFieldConfig<S> {
    /// The label displayed for the secure field.
    label: ControlLabel<S>,
    /// The binding to the secure value being edited.
    value: Binding<Secure>,
}

impl<S> SecureFieldConfig<S> {
    /// The label displayed for the secure field.
    #[must_use]
    pub const fn label(&self) -> &ControlLabel<S> {
        &self.label
    }

    /// Sets the `label`.
    pub fn set_label(&mut self, label: ControlLabel<S>) {
        self.label = label;
    }

    /// The label displayed for the control, for editing it in place.
    pub const fn label_mut(&mut self) -> &mut ControlLabel<S> {
        &mut self.label
    }

    /// The binding to the secure value being edited.
    #[must_use]
    pub const fn value(&self) -> &Binding<Secure> {
        &self.value
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(self) -> (ControlLabel<S>, Binding<Secure>) {
        (self.label, self.value)
    }
}

impl<S: 'static> NativeView for SecureFieldConfig<S> {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

impl<S: 'static> MapSlots<S> for SecureFieldConfig<S> {
    type With<T: 'static> = SecureFieldConfig<T>;

    fn map_slots<T: 'static>(self, f: impl Fn(S) -> T + Clone + 'static) -> SecureFieldConfig<T> {
        SecureFieldConfig {
            label: self.label.map_slots(f),
            value: self.value,
        }
    }
}

/// A secure text entry field for passwords and sensitive data.
///
/// `SecureField` masks input and securely stores values with automatic memory zeroing.
///
/// # Layout Behavior
///
/// `SecureField` **expands horizontally** to fill available space, but has a fixed height.
/// In an `HStack`, it will take up all remaining width after other views are sized.
#[derive(Debug, Clone)]
#[must_use]
pub struct SecureField<L> {
    label: Label<L>,
    value: Binding<Secure>,
}

impl<L> SecureField<L> {
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
    pub fn new<I: IntoLabel<Kind = L>>(label: I, value: &Binding<Secure>) -> Self {
        Self {
            label: label.into_label(),
            value: value.clone(),
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }
}

impl<L: LabelKind> SecureField<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> SecureFieldConfig<S> {
        SecureFieldConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
        }
    }
}

impl<L: LabelKind> SecureField<L> {
    /// Returns the configuration for this secure field.
    #[must_use]
    pub fn config(self) -> SecureFieldConfig<Label<L>> {
        SecureFieldConfig {
            label: self.label.into_unscoped_control_label(),
            value: self.value,
        }
    }
}

impl<S: View> From<SecureFieldConfig<S>> for SecureField<ControlLabel<S>> {
    fn from(value: SecureFieldConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            value: value.value,
        }
    }
}

impl<L: LabelKind> View for SecureField<L> {
    fn body(self, env: &Environment) -> impl View {
        Native::new(self.into_config::<AnyView>(env))
    }

    fn dispatch<B: Backend>(self, backend: &mut B, env: &Environment, cx: B::Ctx) -> B::Output {
        backend.native(self.into_config::<B::Slot>(env), env, cx)
    }
}

impl_label_style_methods!(SecureField<L>);

/// Creates a new `SecureField` instance.
/// See [`SecureField::new`] for more details.
pub fn secure<L: IntoLabel>(label: L, value: &Binding<Secure>) -> SecureField<L::Kind> {
    SecureField::new(label, value)
}
