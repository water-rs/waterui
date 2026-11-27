//! Shared keyboard, capitalization, autocorrection, submit-key, and autofill metadata for text input.

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
