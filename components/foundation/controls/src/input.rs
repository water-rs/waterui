//! Shared keyboard and autofill metadata for text input.

#[derive(Debug, Default)]
#[non_exhaustive]
/// Enum representing the type of keyboard to use for text input.
pub enum KeyboardType {
    #[default]
    /// Default keyboard type, typically used for general text input.
    Text,
    /// Keyboard for email input, which may include special characters like `@` and `.`
    Email,
    /// Keyboard for URL input, which may include characters like `:`, `/`, and `.`
    URL,
    /// Keyboard for numeric input, which may include digits and a decimal point.
    Number,
    /// Keyboard for phone number input, which may include digits and special characters like `+`, `-`, and `()`.
    PhoneNumber,
}

/// The semantic content type a text field declares: what the field's content
/// *means*, for autofill and password/one-time-code services.
///
/// This is the shared vocabulary of Apple's text content types and Android's
/// autofill hints. On Apple platforms it is what makes the system offer a
/// received SMS code above the keyboard; on Android it hands the autofill
/// service the matching hint rather than guessing from the label's language.
/// It is a semantic attribute of the field, not a keyboard layout — a
/// one-time-code field typically pairs it with `KeyboardType::Number`.
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
