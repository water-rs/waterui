use core::{error::Error, fmt::{Display}, str::FromStr};

use alloc::{string::ToString, vec::Vec};
use inkjet::{theme::Theme, tree_sitter_highlight::HighlightEvent};
use nami::impl_constant;
use waterui_color::Srgb;
use waterui_core::Str;

use crate::styled::{Style, StyledStr, ToStyledStr};

/// A trait for syntax highlighting implementations.
pub trait Highlighter: Send + Sync {
    /// Highlights the given text and returns a vector of chunks with colors.
    fn highlight<'a>(&mut self, language:Language,text: &'a str) -> Vec<HighlightChunk<'a>>;
}

/// Highlights text asynchronously using the given highlighter.
#[allow(clippy::unused_async)]
pub async fn highlight_text(language:Language,text: Str,mut highlighter: impl Highlighter) -> StyledStr {
    // TODO: use async thread pool
    highlighter.highlight(language,&text).into_iter().fold(
        StyledStr::empty(),
        |mut s, chunk| { s.push(chunk.text.to_string(), Style::default().foreground(chunk.color)); s },
    )
}

macro_rules! languages {
    ($($ident:ident),*) => {
        /// Supported programming languages for syntax highlighting.
        #[derive(Debug, Clone,PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[non_exhaustive]
        pub enum Language {
            $(
                #[doc = stringify!($ident)]
                $ident,
            )*
        }

        impl From<Language> for inkjet::Language {
            fn from(lang: Language) -> Self {
                match lang {
                    Language::Plaintext => inkjet::Language::Plaintext,
                    Language::Ada => inkjet::Language::Ada,
                    Language::Asm => inkjet::Language::Asm,
                    Language::Awk => inkjet::Language::Awk,
                    Language::Bash => inkjet::Language::Bash,
                    Language::Bibtex => inkjet::Language::Bibtex,
                    Language::Bicep => inkjet::Language::Bicep,
                    Language::Blueprint => inkjet::Language::Blueprint,
                    Language::C => inkjet::Language::C,
                    Language::Capnp => inkjet::Language::Capnp,
                    Language::Clojure => inkjet::Language::Clojure,
                    Language::CSharp => inkjet::Language::CSharp,
                    Language::Cpp => inkjet::Language::Cpp,
                    Language::Css => inkjet::Language::Css,
                    Language::Cue => inkjet::Language::Cue,
                    Language::D => inkjet::Language::D,
                    Language::Dart => inkjet::Language::Dart,
                    Language::Diff => inkjet::Language::Diff,
                    Language::Dockerfile => inkjet::Language::Dockerfile,
                    Language::Eex => inkjet::Language::Eex,
                    Language::Elisp => inkjet::Language::Elisp,
                    Language::Elixir => inkjet::Language::Elixir,
                    Language::Elm => inkjet::Language::Elm,
                    Language::Erlang => inkjet::Language::Erlang,
                    Language::Forth => inkjet::Language::Forth,
                    Language::Fortran => inkjet::Language::Fortran,
                    Language::Fish => inkjet::Language::Fish,
                    Language::Gdscript => inkjet::Language::Gdscript,
                    Language::Gleam => inkjet::Language::Gleam,
                    Language::Glsl => inkjet::Language::Glsl,
                    Language::Go => inkjet::Language::Go,
                    Language::Haskell => inkjet::Language::Haskell,
                    Language::Hcl => inkjet::Language::Hcl,
                    Language::Heex => inkjet::Language::Heex,
                    Language::Html => inkjet::Language::Html,
                    Language::Ini => inkjet::Language::Ini,
                    Language::Java => inkjet::Language::Java,
                    Language::Javascript => inkjet::Language::Javascript,
                    Language::Json => inkjet::Language::Json,
                    Language::Jsx => inkjet::Language::Jsx,
                    Language::Julia => inkjet::Language::Julia,
                    Language::Kotlin => inkjet::Language::Kotlin,
                    Language::Latex => inkjet::Language::Latex,
                    Language::Llvm => inkjet::Language::Llvm,
                    Language::Lua => inkjet::Language::Lua,
                    Language::Make => inkjet::Language::Make,
                    Language::Matlab => inkjet::Language::Matlab,
                    Language::Meson => inkjet::Language::Meson,
                    Language::Nix => inkjet::Language::Nix,
                    Language::ObjectiveC => inkjet::Language::ObjectiveC,
                    Language::Ocaml => inkjet::Language::Ocaml,
                    Language::OcamlInterface => inkjet::Language::OcamlInterface,
                    Language::OpenScad => inkjet::Language::OpenScad,
                    Language::Pascal => inkjet::Language::Pascal,
                    Language::Php => inkjet::Language::Php,
                    Language::ProtoBuf => inkjet::Language::ProtoBuf,
                    Language::Python => inkjet::Language::Python,
                    Language::R => inkjet::Language::R,
                    Language::Racket => inkjet::Language::Racket,
                    Language::Regex => inkjet::Language::Regex,
                    Language::Ruby => inkjet::Language::Ruby,
                    Language::Rust => inkjet::Language::Rust,
                    Language::Scala => inkjet::Language::Scala,
                    Language::Scheme => inkjet::Language::Scheme,
                    Language::Scss => inkjet::Language::Scss,
                    Language::Sql => inkjet::Language::Sql,
                    Language::Svelte => inkjet::Language::Svelte,
                    Language::Swift => inkjet::Language::Swift,
                    Language::Toml => inkjet::Language::Toml,
                    Language::Typescript => inkjet::Language::Typescript,
                    Language::Tsx => inkjet::Language::Tsx,
                    Language::Vimscript => inkjet::Language::Vimscript,
                    Language::Wast => inkjet::Language::Wast,
                    Language::Wat => inkjet::Language::Wat,
                    Language::X86asm => inkjet::Language::X86asm,
                    Language::Wgsl => inkjet::Language::Wgsl,
                    Language::Yaml => inkjet::Language::Yaml,
                    Language::Zig => inkjet::Language::Zig,
                }
            }
        }

        impl Language{
            const fn from_inkjet(lang:inkjet::Language) -> Self {
                match lang {
                    $(inkjet::Language::$ident => Language::$ident,)*
                    _ => Language::Plaintext,
                }
            }
        }

        impl core::fmt::Display for Language {
            fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                match self {
                    $(
                        Self::$ident => write!(f, stringify!($ident)),     
                    )*
                }
            }
        }
    };
}

languages!(Plaintext,Ada,Asm,Awk,Bash,Bibtex,Bicep,Blueprint,C,Capnp,Clojure,CSharp,Cpp,Css,Cue,D,Dart,Diff,Dockerfile,Eex,Elisp,Elixir,Elm,Erlang,Forth,Fortran,Fish,Gdscript,Gleam,Glsl,Go,Haskell,Hcl,Heex,Html,Ini,Java,Javascript,Json,Jsx,Julia,Kotlin,Latex,Llvm,Lua,Make,Matlab,Meson,Nix,ObjectiveC,Ocaml,OcamlInterface,OpenScad,Pascal,Php,ProtoBuf,Python,R,Racket,Regex,Ruby,Rust,Scala,Scheme,Scss,Sql,Svelte,Swift,Toml,Typescript,Tsx,Vimscript,Wast,Wat,X86asm,Wgsl,Yaml,Zig
);

impl_constant!(Language);

#[derive(Debug)]
pub struct ParseLanguageError;
impl Display for ParseLanguageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Failed to parse language")
    }
}
impl Error for ParseLanguageError {}

impl FromStr for Language{
    type Err = ParseLanguageError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        inkjet::Language::from_token(s).ok_or(ParseLanguageError).map(Self::from_inkjet)
    }
}

pub struct DefaultHighlighter(inkjet::Highlighter);

impl Default for DefaultHighlighter {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultHighlighter{
    #[must_use] 
    pub fn new() -> Self {
        Self( inkjet::Highlighter::new())
    }
}

impl Highlighter for DefaultHighlighter {
    fn highlight<'a>(&mut self, language:Language,text: &'a str) -> Vec<HighlightChunk<'a>> {
        let iter=self.0
            .highlight_raw(language.into(), &text).expect("Fail to highlight text");

        let theme = Theme::from_helix(inkjet::theme::vendored::ONEDARK).expect("Fail to load theme");

        let mut chunks = Vec::new();
        let mut current_color = Srgb::new_u8(theme.fg.r, theme.fg.g, theme.fg.b);
        let mut color_stack = Vec::new();

        for event in iter {
            let event = event.expect("Fail to highlight text");
            match event {
                HighlightEvent::Source { start, end } => {
                    let chunk_text = &text[start..end];
                    chunks.push(HighlightChunk {
                        text: chunk_text,
                        color: current_color,
                    });
                },
                HighlightEvent::HighlightStart(highlight) => {
                    // Push current color to stack
                    color_stack.push(current_color);
                    
                    // Get the highlight name and style from theme
                    let name = inkjet::constants::HIGHLIGHT_NAMES[highlight.0];
                    if let Some(style) = theme.get_style(name) {
                        // Use the foreground color if available, otherwise use theme default
                        let color = style.fg.unwrap_or(theme.fg);
                        current_color = Srgb::new_u8(color.r, color.g, color.b);
                    }
                },
                HighlightEvent::HighlightEnd => {
                    // Restore previous color from stack
                    if let Some(color) = color_stack.pop() {
                        current_color = color;
                    }
                },
            }
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
        self.text.to_string().foreground(self.color)
    }
}
