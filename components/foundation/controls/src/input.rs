//! Shared keyboard, capitalization, autocorrection, submit-key, and autofill metadata for text input.

use waterui_core::Environment;
use waterui_core::handler::{Handler, SharedAction};
use waterui_text::{IntoText, Text, TextConfig};

/// Shared authoring-time input configuration carried by both
/// `TextField` and `SecureField`: placeholder prompt, autofill content type,
/// software-keyboard Return-key label, and Return/Enter submit handler.
#[derive(Debug, Clone, Default)]
pub struct FieldInput {
    prompt: Text,
    content_type: Option<ContentType>,
    submit_label: SubmitLabel,
    on_submit: Option<SharedAction>,
}

impl FieldInput {
    /// Creates an empty input configuration with no content type, empty prompt,
    /// [`SubmitLabel::Return`], and no submit handler.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an input configuration with a default [`ContentType`].
    #[must_use]
    pub fn with_content_type(content_type: ContentType) -> Self {
        Self {
            content_type: Some(content_type),
            ..Self::default()
        }
    }

    /// Sets the placeholder prompt shown when the field is empty.
    pub fn set_prompt(&mut self, prompt: impl IntoText) {
        self.prompt = prompt.into_text();
    }

    /// Sets the semantic content type declared for autofill.
    pub const fn set_content_type(&mut self, content_type: ContentType) {
        self.content_type = Some(content_type);
    }

    /// Sets the label and action shown on the software keyboard's Return key.
    pub const fn set_submit_label(&mut self, label: SubmitLabel) {
        self.submit_label = label;
    }

    /// Sets the handler run when the user submits the field with Return or Enter.
    pub fn set_on_submit<Args>(&mut self, action: impl Handler<Args, ()>) {
        self.on_submit = Some(SharedAction::new(action));
    }

    /// Returns the declared semantic content type, if any.
    #[must_use]
    pub const fn content_type(&self) -> Option<ContentType> {
        self.content_type
    }

    /// Returns the configured Return-key label.
    #[must_use]
    pub const fn submit_label(&self) -> SubmitLabel {
        self.submit_label
    }

    /// Resolves the prompt against `env` into a backend-facing [`ResolvedFieldInput`].
    #[must_use]
    pub fn resolve(self, env: &Environment) -> ResolvedFieldInput {
        ResolvedFieldInput {
            prompt: self.prompt.resolve(env),
            content_type: self.content_type,
            submit_label: self.submit_label,
            on_submit: self.on_submit,
        }
    }

    /// Resolves the prompt against a fresh default [`Environment`].
    #[must_use]
    pub fn into_unscoped(self) -> ResolvedFieldInput {
        self.resolve(&Environment::new())
    }
}

/// Resolved input configuration shared by `TextFieldConfig` and `SecureFieldConfig`.
#[derive(Debug, Clone)]
pub struct ResolvedFieldInput {
    prompt: TextConfig,
    content_type: Option<ContentType>,
    submit_label: SubmitLabel,
    on_submit: Option<SharedAction>,
}

impl ResolvedFieldInput {
    /// The resolved placeholder text shown when the field is empty.
    #[must_use]
    pub const fn prompt(&self) -> &TextConfig {
        &self.prompt
    }

    /// The semantic content type the field declares for autofill.
    #[must_use]
    pub const fn content_type(&self) -> Option<ContentType> {
        self.content_type
    }

    /// The label and action shown on the software keyboard's Return key.
    #[must_use]
    pub const fn submit_label(&self) -> SubmitLabel {
        self.submit_label
    }

    /// The action run when the user submits the field with Return or Enter.
    #[must_use]
    pub const fn on_submit(&self) -> Option<&SharedAction> {
        self.on_submit.as_ref()
    }

    /// Takes the resolved input apart into `(prompt, content_type, submit_label, on_submit)`.
    #[must_use]
    pub fn into_parts(
        self,
    ) -> (
        TextConfig,
        Option<ContentType>,
        SubmitLabel,
        Option<SharedAction>,
    ) {
        (
            self.prompt,
            self.content_type,
            self.submit_label,
            self.on_submit,
        )
    }
}

impl From<ResolvedFieldInput> for FieldInput {
    fn from(value: ResolvedFieldInput) -> Self {
        Self {
            prompt: Text::from(value.prompt),
            content_type: value.content_type,
            submit_label: value.submit_label,
            on_submit: value.on_submit,
        }
    }
}

/// Enum representing the type of keyboard to use for text input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum KeyboardType {
    #[default]
    /// Default keyboard type, typically used for general text input.
    Text,
    /// Keyboard for email input, which may include special characters like `@` and `.`.
    Email,
    /// Keyboard for URL input, which may include characters like `:`, `/`, and `.`.
    #[cfg_attr(feature = "serde", serde(rename = "url"))]
    URL,
    /// Keyboard for integer numeric input (digits only).
    Number,
    /// Keyboard for decimal numeric input (digits and the locale's decimal separator).
    Decimal,
    /// Keyboard for phone number input, which may include digits and special characters like `+`, `-`, and `()`.
    PhoneNumber,
}

impl KeyboardType {
    /// Default autocapitalization behavior implied by this keyboard layout.
    #[must_use]
    pub const fn default_capitalization(self) -> Capitalization {
        match self {
            Self::Text => Capitalization::Sentences,
            Self::Email | Self::URL | Self::Number | Self::Decimal | Self::PhoneNumber => {
                Capitalization::None
            }
        }
    }

    /// Default autocorrection behavior implied by this keyboard layout.
    #[must_use]
    pub const fn default_autocorrect(self) -> bool {
        matches!(self, Self::Text)
    }
}

/// How the input system capitalizes typed text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum Capitalization {
    /// Do not capitalize automatically.
    None,
    /// Capitalize the first letter of each word.
    Words,
    /// Capitalize the first letter of each sentence.
    #[default]
    Sentences,
    /// Capitalize every character.
    Characters,
}

/// The label and action shown on the software keyboard's Return key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum SubmitLabel {
    /// The platform's default Return key.
    #[default]
    Return,
    /// "Done".
    Done,
    /// "Go".
    Go,
    /// "Next".
    Next,
    /// "Search".
    Search,
    /// "Send".
    Send,
    /// "Join".
    Join,
    /// "Continue".
    Continue,
}

/// The semantic content type a text field declares: what the field's content
/// *means*, for autofill and password/one-time-code services.
///
/// This is the shared vocabulary of Apple's text content types and Android's
/// autofill hints. On Apple platforms it is what makes the system offer a
/// received SMS code above the keyboard; on Android it hands the autofill
/// service the matching hint rather than guessing from the label's language.
/// It also drives default keyboard, capitalization, and autocorrection
/// behavior unless explicitly overridden on the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[expect(
    clippy::exhaustive_enums,
    reason = "backends realize each content type variant by variant; a new content type must fail their build instead of falling into a wildcard arm"
)]
pub enum ContentType {
    /// A username or account identifier.
    Username,
    /// An existing password.
    Password,
    /// A new password, in account creation or password change.
    NewPassword,
    /// An email address.
    EmailAddress,
    /// A telephone number.
    PhoneNumber,
    /// A one-time code delivered out of band — an SMS OTP and similar.
    OneTimeCode,
    /// A person's full name.
    PersonName,
    /// A full street address.
    PostalAddress,
    /// A postal or ZIP code.
    PostalCode,
    /// A payment card number.
    CreditCardNumber,
}

impl ContentType {
    /// Default keyboard layout implied by this content type.
    #[must_use]
    pub const fn default_keyboard(self) -> KeyboardType {
        match self {
            Self::EmailAddress => KeyboardType::Email,
            Self::PhoneNumber => KeyboardType::PhoneNumber,
            Self::CreditCardNumber => KeyboardType::Number,
            Self::Username
            | Self::Password
            | Self::NewPassword
            | Self::OneTimeCode
            | Self::PersonName
            | Self::PostalAddress
            | Self::PostalCode => KeyboardType::Text,
        }
    }

    /// Default autocapitalization behavior implied by this content type.
    #[must_use]
    pub const fn default_capitalization(self) -> Capitalization {
        match self {
            Self::PersonName | Self::PostalAddress => Capitalization::Words,
            Self::PostalCode => Capitalization::Characters,
            Self::Username
            | Self::Password
            | Self::NewPassword
            | Self::EmailAddress
            | Self::PhoneNumber
            | Self::OneTimeCode
            | Self::CreditCardNumber => Capitalization::None,
        }
    }

    /// Default autocorrection behavior implied by this content type.
    #[must_use]
    pub const fn default_autocorrect(self) -> bool {
        match self {
            Self::Username
            | Self::Password
            | Self::NewPassword
            | Self::EmailAddress
            | Self::PhoneNumber
            | Self::OneTimeCode
            | Self::PersonName
            | Self::PostalAddress
            | Self::PostalCode
            | Self::CreditCardNumber => false,
        }
    }
}
