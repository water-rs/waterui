use core::{
    error::Error,
    fmt::{Debug, Display},
    str::FromStr,
};

use alloc::{string::ToString, vec::Vec};
use nami::impl_constant;
use syntect::{
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};
use two_face::syntax::extra_newlines;
use waterui_color::Srgb;
use waterui_core::Str;

use crate::styled::{Style, StyledStr};

/// A trait for syntax highlighting implementations.
pub trait Highlighter: Send + Sync {
    /// Highlights the given text and returns a vector of chunks with colors.
    fn highlight<'a>(&mut self, language: Language, text: &'a str) -> Vec<HighlightChunk<'a>>;
}

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

macro_rules! languages {
    ($($ident:ident => $ext:literal),* $(,)?) => {
        /// Supported programming languages for syntax highlighting.
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Language {
            $(
                #[doc = stringify!($ident)]
                $ident,
            )*
        }

        impl Language {
            /// Returns the file extension associated with this language.
            #[must_use]
            pub const fn extension(&self) -> &'static str {
                match self {
                    $(Self::$ident => $ext,)*
                }
            }

            /// Returns the token name for this language (lowercase).
            #[must_use]
            pub const fn token(&self) -> &'static str {
                match self {
                    $(Self::$ident => const {
                        const fn to_lower(s: &str) -> &str { s }
                        to_lower(stringify!($ident))
                    },)*
                }
            }
        }

        impl core::fmt::Display for Language {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    $(Self::$ident => write!(f, stringify!($ident)),)*
                }
            }
        }

        impl FromStr for Language {
            type Err = ParseLanguageError;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let s_lower = s.to_lowercase();
                $(
                    if s_lower == stringify!($ident).to_lowercase() || s_lower == $ext {
                        return Ok(Self::$ident);
                    }
                )*
                // Additional aliases
                match s_lower.as_str() {
                    "c++" | "cxx" => Ok(Self::Cpp),
                    "c#" => Ok(Self::CSharp),
                    "obj-c" | "objc" => Ok(Self::ObjectiveC),
                    "shell" => Ok(Self::Bash),
                    "yml" => Ok(Self::Yaml),
                    "text" => Ok(Self::Plaintext),
                    _ => Err(ParseLanguageError),
                }
            }
        }
    };
}

languages!(
    Plaintext => "txt",
    Bash => "sh",
    C => "c",
    Cpp => "cpp",
    CSharp => "cs",
    Css => "css",
    Clojure => "clj",
    D => "d",
    Diff => "diff",
    Erlang => "erl",
    Go => "go",
    Haskell => "hs",
    Html => "html",
    Java => "java",
    Javascript => "js",
    Json => "json",
    Kotlin => "kt",
    Latex => "tex",
    Lisp => "lisp",
    Lua => "lua",
    Makefile => "makefile",
    Markdown => "md",
    ObjectiveC => "m",
    OCaml => "ml",
    Pascal => "pas",
    Perl => "pl",
    Php => "php",
    Python => "py",
    R => "r",
    Ruby => "rb",
    Rust => "rs",
    Scala => "scala",
    Sql => "sql",
    Swift => "swift",
    Toml => "toml",
    Typescript => "ts",
    Xml => "xml",
    Yaml => "yaml",
    Zig => "zig",
);

impl_constant!(Language);

/// Error returned when a language token cannot be parsed.
#[derive(Debug)]
pub struct ParseLanguageError;

impl Display for ParseLanguageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Failed to parse language")
    }
}

impl Error for ParseLanguageError {}

/// Default syntax highlighter implementation using the syntect library.
pub struct DefaultHighlighter {
    syntax_set: SyntaxSet,
    theme: Theme,
}

impl Debug for DefaultHighlighter {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DefaultHighlighter").finish()
    }
}

impl Default for DefaultHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultHighlighter {
    /// Creates a new highlighter backed by syntect with extended syntax support.
    #[must_use]
    pub fn new() -> Self {
        // Use two-face's extended syntax set which includes Swift and many more languages
        let syntax_set = extra_newlines();
        let theme_set = ThemeSet::load_defaults();
        let theme = theme_set.themes["base16-ocean.dark"].clone();
        Self { syntax_set, theme }
    }

    fn find_syntax(&self, language: &Language) -> &SyntaxReference {
        self.syntax_set
            .find_syntax_by_extension(language.extension())
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
    }
}

impl Highlighter for DefaultHighlighter {
    fn highlight<'a>(&mut self, language: Language, text: &'a str) -> Vec<HighlightChunk<'a>> {
        use syntect::easy::HighlightLines;

        let syntax = self.find_syntax(&language);
        let mut h = HighlightLines::new(syntax, &self.theme);
        let mut chunks = Vec::new();

        for line in text.lines() {
            let Ok(ranges) = h.highlight_line(line, &self.syntax_set) else {
                // Fallback: return the whole line with default color
                chunks.push(HighlightChunk {
                    text: line,
                    color: Srgb::new_u8(200, 200, 200),
                });
                continue;
            };

            for (style, text_slice) in ranges {
                let color =
                    Srgb::new_u8(style.foreground.r, style.foreground.g, style.foreground.b);
                chunks.push(HighlightChunk {
                    text: text_slice,
                    color,
                });
            }

            // Add newline back (syntect strips it)
            if text.contains('\n') {
                chunks.push(HighlightChunk {
                    text: "\n",
                    color: Srgb::new_u8(200, 200, 200),
                });
            }
        }

        // Handle trailing content without newline
        if !text.ends_with('\n')
            && let Some(last) = chunks.last_mut()
            && last.text == "\n"
        {
            chunks.pop();
        }

        chunks
    }
}

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
        StyledStr::from(self.text.to_string()).foreground(self.color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swift_syntax_exists() {
        let syntax_set = extra_newlines();
        let swift_syntax = syntax_set.find_syntax_by_extension("swift");
        assert!(
            swift_syntax.is_some(),
            "Swift syntax should exist in two-face"
        );
    }

    #[test]
    fn test_swift_highlighting() {
        let mut highlighter = DefaultHighlighter::new();
        let code = "import SwiftUI\nstruct ContentView: View { }";
        let chunks = highlighter.highlight(Language::Swift, code);
        // Should have multiple chunks with different colors (not all plain text)
        assert!(chunks.len() > 1, "Swift code should be tokenized");
    }
}
