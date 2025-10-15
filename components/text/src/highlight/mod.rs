use alloc::{string::ToString, vec::Vec};
use core::{
    error::Error,
    fmt::{self, Display},
};

use crate::styled::{Style, StyledStr, ToStyledStr};
use waterui_color::Srgb;
use waterui_core::Str;

/// A trait for syntax highlighting implementations.
pub trait Highlighter: Send + Sync {
    /// Highlights the given text and returns a vector of chunks with colors.
    fn highlight<'a>(&mut self, language: Language, text: &'a str) -> Vec<HighlightChunk<'a>>;
}

/// Error returned when a language token cannot be parsed.
#[derive(Debug)]
pub struct ParseLanguageError;

impl Display for ParseLanguageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to parse language")
    }
}

impl Error for ParseLanguageError {}

/// A chunk of highlighted text with an associated color.
#[derive(Debug)]
pub struct HighlightChunk<'a> {
    /// The text content.
    pub text: &'a str,
    /// The color for this chunk.
    pub color: Srgb,
}

impl HighlightChunk<'_> {
    /// Converts this chunk into a styled string.
    #[must_use]
    pub fn attributed(self) -> StyledStr {
        self.text.to_string().foreground(self.color)
    }
}

macro_rules! languages {
    ( $( $variant:ident => [$($token:expr),* $(,)?] ),* $(,)? ) => {
        /// Supported programming languages for syntax highlighting.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Language {
            $(
                #[doc = stringify!($variant)]
                $variant,
            )*
        }

        impl Language {
            #[inline]
            fn from_token(token: impl AsRef<str>) -> Option<Self> {
                let normalized = token.as_ref().trim().to_ascii_lowercase();
                match normalized.as_str() {
                    $(
                        $( $token => Some(Self::$variant), )*
                    )*
                    _ => None,
                }
            }
        }

        impl ::core::str::FromStr for Language {
            type Err = $crate::highlight::ParseLanguageError;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Self::from_token(s).ok_or($crate::highlight::ParseLanguageError)
            }
        }

        impl ::core::fmt::Display for Language {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(
                        Self::$variant => write!(f, stringify!($variant)),
                    )*
                }
            }
        }

        #[cfg(not(target_arch = "wasm32"))]
        impl From<Language> for ::inkjet::Language {
            fn from(lang: Language) -> Self {
                match lang {
                    $( Language::$variant => ::inkjet::Language::$variant, )*
                }
            }
        }

    };
}

pub(crate) use languages;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use native::*;
#[cfg(target_arch = "wasm32")]
pub use wasm::*;

/// Highlights text asynchronously using the given highlighter.
#[allow(clippy::unused_async)]
pub async fn highlight_text(
    language: Language,
    text: Str,
    mut highlighter: impl Highlighter,
) -> StyledStr {
    // TODO: use async thread pool
    highlighter
        .highlight(language, &text)
        .into_iter()
        .fold(StyledStr::empty(), |mut s, chunk| {
            s.push(
                chunk.text.to_string(),
                Style::default().foreground(chunk.color),
            );
            s
        })
}
