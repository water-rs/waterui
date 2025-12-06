//! Error types for URL parsing.

use core::fmt;

/// Error type returned when URL parsing fails.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    kind: ParseErrorKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseErrorKind {
    /// The URL string is empty
    Empty,
}

impl ParseError {
    /// Creates a new `ParseError` for empty URL strings.
    pub(crate) const fn empty() -> Self {
        Self {
            kind: ParseErrorKind::Empty,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.kind {
            ParseErrorKind::Empty => write!(f, "URL string is empty"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "std")]
    #[test]
    fn test_parse_error_display() {
        use alloc::string::ToString;
        let error = ParseError::empty();
        assert_eq!(error.to_string(), "URL string is empty");
    }

    #[test]
    fn test_parse_error_debug() {
        use alloc::format;
        let error = ParseError::empty();
        assert_eq!(format!("{error:?}"), "ParseError { kind: Empty }");
    }

    #[test]
    fn test_parse_error_equality() {
        let error1 = ParseError::empty();
        let error2 = ParseError::empty();
        assert_eq!(error1, error2);
    }
}
