use core::error::Error;

use waterui_core::View;
use waterui_layout::{
    scroll, spacer,
    stack::{hstack, vstack},
};
use waterui_str::Str;
use waterui_text::{
    highlight::{DefaultHighlighter, Language, highlight_text},
    text,
};

use crate::widget::suspense::suspense;

/// View that renders syntax-highlighted code snippets.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Code {
    language: Language,
    content: Str,
}

impl Code {
    /// Creates a new `Code` view for the provided language and content.
    ///
    /// # Panics
    ///
    /// Panics if the language cannot be converted into a supported [`Language`].
    pub fn new(language: impl TryInto<Language, Error: Error>, content: impl Into<Str>) -> Self {
        Self {
            language: language.try_into().expect("Invalid language"),
            content: content.into(),
        }
    }
}

impl View for Code {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        scroll(vstack((
            hstack((text!("{}", self.language).bold(), spacer(), text("Copy"))),
            suspense(highlight_text(
                self.language,
                self.content,
                DefaultHighlighter::new(),
            )),
        )))
    }
}

/// Convenience constructor for creating a [`Code`] view inline.
pub fn code(language: impl TryInto<Language, Error: Error>, content: impl Into<Str>) -> Code {
    Code::new(language, content)
}
