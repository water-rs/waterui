use core::fmt;
use core::{
    error::Error,
    fmt::{Debug, Display},
    str::FromStr,
};

use alloc::{string::ToString, vec::Vec};
use nami::impl_constant;
#[cfg(feature = "highlight")]
use syntect::{
    highlighting::{Theme, ThemeSet},
    parsing::{SyntaxReference, SyntaxSet},
};
#[cfg(feature = "highlight")]
use two_face::syntax::extra_newlines;
#[cfg(feature = "highlight")]
use waterui_graphics::color::ColorScheme;
use waterui_graphics::color::Srgb;

use crate::styled::{Style, StyledStr};

/// A trait for syntax highlighting implementations.
pub trait Highlighter: Send + Sync {
    /// Highlights the given text and returns a vector of chunks with colors.
    fn highlight<'a>(&mut self, language: Language, text: &'a str) -> Vec<HighlightChunk<'a>>;
}

/// Highlights text using the given highlighter.
pub fn highlight_text(
    language: Language,
    text: &str,
    highlighter: &mut impl Highlighter,
) -> StyledStr {
    // TODO: use async thread pool
    highlighter
        .highlight(language, text)
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

        impl fmt::Display for Language {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Failed to parse language")
    }
}

impl Error for ParseLanguageError {}

/// Lets [`code`](crate::code) take the fence's token as written — `"rust"`,
/// `"c++"`, `"yml"` — through its `TryInto<Language>` argument.
impl TryFrom<&str> for Language {
    type Error = ParseLanguageError;

    fn try_from(token: &str) -> Result<Self, Self::Error> {
        token.parse()
    }
}

/// Default syntax highlighter implementation using the syntect library.
#[cfg(feature = "highlight")]
pub struct DefaultHighlighter {
    syntax_set: SyntaxSet,
    theme: Theme,
    /// The theme's own text colour, for chunks the grammar assigns no scope.
    foreground: Srgb,
}

#[cfg(feature = "highlight")]
impl Debug for DefaultHighlighter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DefaultHighlighter").finish()
    }
}

#[cfg(feature = "highlight")]
impl DefaultHighlighter {
    /// Creates a highlighter backed by syntect with extended syntax support,
    /// using the palette that reads against a light or a dark surface.
    ///
    /// # Panics
    ///
    /// Panics if the bundled syntect theme declares no default foreground.
    #[must_use]
    pub fn new(scheme: ColorScheme) -> Self {
        // Use two-face's extended syntax set which includes Swift and many more languages
        let syntax_set = extra_newlines();
        let theme_set = ThemeSet::load_defaults();
        let name = match scheme {
            ColorScheme::Light => "base16-ocean.light",
            ColorScheme::Dark => "base16-ocean.dark",
        };
        let theme = theme_set.themes[name].clone();
        let foreground = theme
            .settings
            .foreground
            .map(|color| Srgb::new_u8(color.r, color.g, color.b))
            .expect("syntect theme declares no default foreground");
        Self {
            syntax_set,
            theme,
            foreground,
        }
    }

    fn find_syntax(&self, language: &Language) -> &SyntaxReference {
        self.syntax_set
            .find_syntax_by_extension(language.extension())
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text())
    }
}

#[cfg(feature = "highlight")]
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
                    color: self.foreground,
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
                    color: self.foreground,
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

#[cfg(all(test, feature = "highlight"))]
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
        let mut highlighter = DefaultHighlighter::new(ColorScheme::Dark);
        let code = "import SwiftUI\nstruct ContentView: View { }";
        let chunks = highlighter.highlight(Language::Swift, code);
        // Should have multiple chunks with different colors (not all plain text)
        assert!(chunks.len() > 1, "Swift code should be tokenized");
    }

    #[test]
    fn palette_follows_the_colour_scheme() {
        let code = "import SwiftUI\nstruct ContentView: View { }";
        let colours = |scheme| {
            DefaultHighlighter::new(scheme)
                .highlight(Language::Swift, code)
                .into_iter()
                .map(|chunk| chunk.color)
                .collect::<Vec<_>>()
        };
        assert_ne!(
            colours(ColorScheme::Light),
            colours(ColorScheme::Dark),
            "a light surface and a dark surface need different ink"
        );
    }
}
