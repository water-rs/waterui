//! `Language::token()` is documented as lowercase.

#![cfg(feature = "highlight")]

use waterui_text::highlight::Language;

#[test]
fn token_is_the_lowercase_language_name() {
    assert_eq!(Language::CSharp.token(), "csharp");
    assert_eq!(Language::Cpp.token(), "cpp");
}
