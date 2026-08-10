//! Error types for URL parsing.

use core::fmt;

#[cfg(feature = "std")]
use core::error::Error;

/// Error type returned when URL parsing fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    kind: ParseErrorKind,
}

/// Why a URL string could not be parsed.
///
/// The parser is a `const fn` so that [`Url::new`](crate::Url::new) can validate
/// literals at compile time. It reports failures as this enum rather than
/// panicking, and the panicking `const` entry point turns each variant into a
/// literal compile-time message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseErrorKind {
    /// The URL string is empty.
    Empty,
    /// The URL string exceeds the 65535-byte limit imposed by the compact span
    /// representation.
    TooLong,
    /// A hierarchical URL had no scheme before `://`.
    MissingScheme,
    /// A hierarchical URL that requires an authority had no host.
    MissingHost,
    /// The port was empty, too long, or contained non-digits.
    InvalidPort,
}

impl ParseError {
    pub(crate) const fn new(kind: ParseErrorKind) -> Self {
        Self { kind }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.kind.message())
    }
}

impl ParseErrorKind {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Empty => "URL string is empty",
            Self::TooLong => "URL strings longer than 65535 bytes are not supported",
            Self::MissingScheme => "web URL must have a scheme",
            Self::MissingHost => "web URL must have a host",
            Self::InvalidPort => "web URL has an invalid port",
        }
    }
}

#[cfg(feature = "std")]
impl Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "std")]
    #[test]
    fn test_parse_error_display() {
        use alloc::string::ToString;
        let error = ParseError::new(ParseErrorKind::Empty);
        assert_eq!(error.to_string(), "URL string is empty");
    }

    #[cfg(feature = "std")]
    #[test]
    fn every_kind_has_a_message() {
        use alloc::string::ToString;
        for kind in [
            ParseErrorKind::Empty,
            ParseErrorKind::TooLong,
            ParseErrorKind::MissingScheme,
            ParseErrorKind::MissingHost,
            ParseErrorKind::InvalidPort,
        ] {
            assert!(!ParseError::new(kind).to_string().is_empty());
        }
    }

    #[test]
    fn test_parse_error_debug() {
        use alloc::format;
        let error = ParseError::new(ParseErrorKind::Empty);
        assert_eq!(format!("{error:?}"), "ParseError { kind: Empty }");
    }

    #[test]
    fn test_parse_error_equality() {
        let error1 = ParseError::new(ParseErrorKind::Empty);
        let error2 = ParseError::new(ParseErrorKind::Empty);
        assert_eq!(error1, error2);
    }
}
