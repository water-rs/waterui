//! Infallible conversion into [`Url`].

use suiteki::Str;
use waterui_contract::contract_violation;

use crate::{Url, parser};

/// Types that already name a URL, converted without a fallible step.
///
/// This is deliberately **not** implemented for `String`, `Str`, or a non-static
/// `&str`. A URL that only exists at runtime can fail to parse, and quietly
/// papering over that is how a malformed address reaches a native web view. Parse
/// runtime text explicitly — [`Url::parse_user_input`] for something a person
/// typed, `str::parse` otherwise — and handle the failure at that point.
///
/// # Examples
///
/// ```
/// use waterui_url::{IntoUrl, Url};
///
/// fn navigate(url: impl IntoUrl) -> Url {
///     url.into_url()
/// }
///
/// // A literal is accepted directly.
/// assert_eq!(navigate("https://waterui.dev").as_str(), "https://waterui.dev");
///
/// // For compile-time validation, build the constant in a `const` context:
/// const HOME: Url = Url::new("https://waterui.dev");
/// assert_eq!(navigate(HOME).as_str(), "https://waterui.dev");
/// ```
///
/// Runtime text does not compile, by design:
///
/// ```compile_fail
/// use waterui_url::IntoUrl;
/// let typed = String::from("https://waterui.dev");
/// let _ = typed.into_url();
/// ```
pub trait IntoUrl {
    /// Converts `self` into a [`Url`].
    fn into_url(self) -> Url;
}

impl IntoUrl for Url {
    fn into_url(self) -> Self {
        self
    }
}

impl IntoUrl for &Url {
    fn into_url(self) -> Url {
        self.clone()
    }
}

impl IntoUrl for &'static str {
    /// A malformed literal is a contract violation: it panics in a debug build,
    /// and a release build logs it and uses `about:blank`. Write the value into
    /// a `const` [`Url::new`] to move that check to compile time.
    fn into_url(self) -> Url {
        match parser::try_parse_url(self.as_bytes()) {
            Ok(components) => Url {
                inner: Str::from_static(self),
                components,
            },
            Err(error) => {
                contract_violation!(
                    "the URL literal {self:?} is malformed: {}; about:blank is used instead",
                    error.message()
                );
                Url::new("about:blank")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::IntoUrl;
    use crate::Url;

    const HOME: Url = Url::new("https://waterui.dev");

    #[test]
    fn literals_and_urls_convert_without_a_fallible_step() {
        assert_eq!(
            "https://waterui.dev/docs".into_url().as_str(),
            "https://waterui.dev/docs"
        );

        assert_eq!(HOME.into_url().as_str(), "https://waterui.dev");
        assert_eq!((&HOME).into_url().as_str(), "https://waterui.dev");
    }

    #[test]
    fn non_web_literals_are_accepted() {
        assert!("about:blank".into_url().is_opaque());
        assert!("file:///tmp/page.html".into_url().is_local());
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "the URL literal \"\" is malformed")
    )]
    fn a_malformed_literal_becomes_about_blank() {
        assert_eq!("".into_url().as_str(), "about:blank");
    }
}
