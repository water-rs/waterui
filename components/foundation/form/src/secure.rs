//! Secure form components for handling sensitive data.
//!
//! This module provides utilities for handling sensitive form data such as
//! passwords and other secrets with automatic memory zeroing for security.

use core::fmt;
use core::{fmt::Debug, str::FromStr};

use alloc::string::{String, ToString};
use nami::Binding;
pub use waterui_controls::input::{ContentType, SubmitLabel};
use waterui_controls::input::{FieldInput, ResolvedFieldInput};
use waterui_controls::label::{ControlLabel, Label, LabelKind};
use waterui_controls::{IntoLabel, impl_label_style_methods};
use waterui_core::backend::{Backend, ChildSlot, MapSlots};
use waterui_core::handler::{Handler, SharedAction};
use waterui_core::layout::StretchAxis;
use waterui_core::{AnyView, Environment, Native, NativeView, View};
use waterui_text::{IntoText, TextConfig};
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
    /// Shared resolved input configuration (prompt, content type, submit label, submit action).
    input: ResolvedFieldInput,
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

    /// The resolved placeholder text shown when the field is empty.
    #[must_use]
    pub const fn prompt(&self) -> &TextConfig {
        self.input.prompt()
    }

    /// The semantic content type the field declares for autofill.
    ///
    /// Defaults to `Some(ContentType::Password)`.
    #[must_use]
    pub const fn content_type(&self) -> Option<ContentType> {
        self.input.content_type()
    }

    /// The label and action shown on the software keyboard's Return key.
    #[must_use]
    pub const fn submit_label(&self) -> SubmitLabel {
        self.input.submit_label()
    }

    /// The action run when the user submits the field with Return or Enter.
    #[must_use]
    pub const fn on_submit(&self) -> Option<&SharedAction> {
        self.input.on_submit()
    }

    /// Takes the value apart into its fields, in declaration order.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        ControlLabel<S>,
        Binding<Secure>,
        TextConfig,
        Option<ContentType>,
        SubmitLabel,
        Option<SharedAction>,
    ) {
        let (prompt, content_type, submit_label, on_submit) = self.input.into_parts();
        (
            self.label,
            self.value,
            prompt,
            content_type,
            submit_label,
            on_submit,
        )
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
            input: self.input,
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
    input: FieldInput,
}

impl<L> SecureField<L> {
    /// Creates a new `SecureField` instance.
    ///
    /// Defaults to [`ContentType::Password`] for autofill.
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
            input: FieldInput::with_content_type(ContentType::Password),
        }
    }

    pub(crate) const fn label_mut(&mut self) -> &mut Label<L> {
        &mut self.label
    }

    /// Sets the placeholder prompt shown when the field is empty.
    pub fn prompt(mut self, prompt: impl IntoText) -> Self {
        self.input.set_prompt(prompt);
        self
    }

    /// Sets the semantic content type declared for autofill (defaults to
    /// [`ContentType::Password`]; pass [`ContentType::NewPassword`] on
    /// sign-up or password-change forms).
    pub const fn content_type(mut self, content_type: ContentType) -> Self {
        self.input.set_content_type(content_type);
        self
    }

    /// Sets the label and action shown on the software keyboard's Return key.
    pub const fn submit_label(mut self, label: SubmitLabel) -> Self {
        self.input.set_submit_label(label);
        self
    }

    /// Runs `action` when the user submits the field with Return or Enter.
    pub fn on_submit<Args>(mut self, action: impl Handler<Args, ()>) -> Self {
        self.input.set_on_submit(action);
        self
    }
}

impl<L: LabelKind> SecureField<L> {
    fn into_config<S: ChildSlot>(self, env: &Environment) -> SecureFieldConfig<S> {
        SecureFieldConfig {
            label: self.label.into_control_label::<S>(env),
            value: self.value,
            input: self.input.resolve(env),
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
            input: self.input.into_unscoped(),
        }
    }
}

impl<S: View> From<SecureFieldConfig<S>> for SecureField<ControlLabel<S>> {
    fn from(value: SecureFieldConfig<S>) -> Self {
        let mode = value.label.display_mode_preference();
        Self {
            label: Label::from_control_label(value.label, mode),
            value: value.value,
            input: value.input.into(),
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

#[cfg(test)]
mod tests {
    use nami::{Binding, Signal};
    use waterui_core::{AnyView, Environment};

    use super::{ContentType, Secure, SecureField, SubmitLabel, secure};

    #[test]
    fn defaults_to_password_content_type_and_return_submit_label() {
        let secret = Binding::new(Secure::default());
        let config = secure("Password", &secret).into_config::<AnyView>(&Environment::new());
        assert_eq!(config.content_type(), Some(ContentType::Password));
        assert_eq!(config.submit_label(), SubmitLabel::Return);
        assert!(config.on_submit().is_none());
        assert_eq!(config.prompt().content().snapshot().to_plain(), "");
    }

    #[test]
    fn modifiers_configure_prompt_content_type_submit_label_and_on_submit() {
        let secret = Binding::new(Secure::default());
        let submitted = Binding::new(false);
        let env = Environment::new();
        let config = SecureField::new("Password", &secret)
            .prompt("At least 12 characters")
            .content_type(ContentType::NewPassword)
            .submit_label(SubmitLabel::Go)
            .on_submit({
                let submitted = submitted.clone();
                move || submitted.set(true)
            })
            .into_config::<AnyView>(&env);

        assert_eq!(
            config.prompt().content().snapshot().to_plain(),
            "At least 12 characters"
        );
        assert_eq!(config.content_type(), Some(ContentType::NewPassword));
        assert_eq!(config.submit_label(), SubmitLabel::Go);
        config
            .on_submit()
            .expect("on_submit handler must be present")
            .run(&env);
        assert!(submitted.snapshot());

        // Round-trip through From<SecureFieldConfig<S>> preserves every input field.
        let rebuilt = SecureField::from(config).into_config::<AnyView>(&env);
        assert_eq!(
            rebuilt.prompt().content().snapshot().to_plain(),
            "At least 12 characters"
        );
        assert_eq!(rebuilt.content_type(), Some(ContentType::NewPassword));
        assert_eq!(rebuilt.submit_label(), SubmitLabel::Go);
        assert!(rebuilt.on_submit().is_some());
    }
}
