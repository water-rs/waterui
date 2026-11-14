//! The semantic content type a text field declares — the shared
//! `UITextContentType` / `NSTextContentType` vocabulary that autofill and
//! the one-time-code suggestion key on.

/// What a text field's content means, for autofill and password/OTP
/// services. Each platform's `TextField::set_content_type` maps it onto the
/// native `UITextContentType` / `NSTextContentType` constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

#[cfg(target_os = "ios")]
impl ContentType {
    /// The `UITextContentType` constant this maps onto.
    pub(crate) fn native(self) -> &'static objc2_ui_kit::UITextContentType {
        use objc2_ui_kit::{
            UITextContentTypeCreditCardNumber, UITextContentTypeEmailAddress,
            UITextContentTypeFullStreetAddress, UITextContentTypeName,
            UITextContentTypeNewPassword, UITextContentTypeOneTimeCode, UITextContentTypePassword,
            UITextContentTypePostalCode, UITextContentTypeTelephoneNumber,
            UITextContentTypeUsername,
        };
        // SAFETY: framework string constants — immutable, alive for the
        // process, never aliased mutably.
        unsafe {
            match self {
                Self::Username => UITextContentTypeUsername,
                Self::Password => UITextContentTypePassword,
                Self::NewPassword => UITextContentTypeNewPassword,
                Self::EmailAddress => UITextContentTypeEmailAddress,
                Self::PhoneNumber => UITextContentTypeTelephoneNumber,
                Self::OneTimeCode => UITextContentTypeOneTimeCode,
                Self::PersonName => UITextContentTypeName,
                Self::PostalAddress => UITextContentTypeFullStreetAddress,
                Self::PostalCode => UITextContentTypePostalCode,
                Self::CreditCardNumber => UITextContentTypeCreditCardNumber,
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl ContentType {
    /// The `NSTextContentType` constant this maps onto.
    pub(crate) fn native(self) -> &'static objc2_app_kit::NSTextContentType {
        use objc2_app_kit::{
            NSTextContentTypeCreditCardNumber, NSTextContentTypeEmailAddress,
            NSTextContentTypeFullStreetAddress, NSTextContentTypeName,
            NSTextContentTypeNewPassword, NSTextContentTypeOneTimeCode, NSTextContentTypePassword,
            NSTextContentTypePostalCode, NSTextContentTypeTelephoneNumber,
            NSTextContentTypeUsername,
        };
        // SAFETY: framework string constants — immutable, alive for the
        // process, never aliased mutably.
        unsafe {
            match self {
                Self::Username => NSTextContentTypeUsername,
                Self::Password => NSTextContentTypePassword,
                Self::NewPassword => NSTextContentTypeNewPassword,
                Self::EmailAddress => NSTextContentTypeEmailAddress,
                Self::PhoneNumber => NSTextContentTypeTelephoneNumber,
                Self::OneTimeCode => NSTextContentTypeOneTimeCode,
                Self::PersonName => NSTextContentTypeName,
                Self::PostalAddress => NSTextContentTypeFullStreetAddress,
                Self::PostalCode => NSTextContentTypePostalCode,
                Self::CreditCardNumber => NSTextContentTypeCreditCardNumber,
            }
        }
    }
}
