use core::error::Error;
use executor_core::spawn_local;
use nami::Binding;
use native_executor::sleep;
use std::time::Duration;
use waterui_color::Color;
use waterui_core::View;
use waterui_layout::{
    spacer,
    stack::{HorizontalAlignment, VStack, hstack},
};
use waterui_str::Str;
use waterui_text::{
    font::{Body, Font},
    highlight::{DefaultHighlighter, Highlighter, Language},
    styled::{Style, StyledStr},
    text,
};

use crate::{SignalExt, ViewExt};

/// Copies text to the system clipboard.
fn copy_to_clipboard(text: &str) {
    #[cfg(not(target_os = "android"))]
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => {
            if let Err(e) = clipboard.set_text(text) {
                tracing::error!("Failed to copy to clipboard: {}", e);
            }
        }
        Err(e) => {
            tracing::error!("Failed to access clipboard: {}", e);
        }
    }

    #[cfg(target_os = "android")]
    {
        if let Err(e) = android_clipboard::set_text(text.to_string()) {
            tracing::error!("Failed to copy to clipboard: {}", e);
        }
    }
}

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
        let lang_name = self.language.to_string();
        let content_for_copy = self.content.to_string();
        let mut highlighter = DefaultHighlighter::new();
        let chunks = highlighter.highlight(self.language, &self.content);

        let code_font = Font::from(Body).size(14.0);
        let styled = chunks.into_iter().fold(StyledStr::empty(), |mut s, chunk| {
            s.push(
                chunk.text.to_string(),
                Style::default()
                    .foreground(chunk.color)
                    .font(code_font.clone()),
            );
            s
        });

        let copied = Binding::container(false);

        // Code block with dark background, left-aligned content
        VStack::new(
            HorizontalAlignment::Leading,
            8.0,
            (
                hstack((
                    text(lang_name)
                        .bold()
                        .foreground(Color::srgb_f32(0.85, 0.86, 0.9)),
                    spacer(),
                    text(copied.select("Copied", "Copy").animated())
                        .foreground(Color::srgb_f32(0.72, 0.74, 0.8))
                        .on_tap(move || {
                            copy_to_clipboard(&content_for_copy);
                            let copied = copied.clone();
                            spawn_local(async move {
                                copied.set(true);
                                sleep(Duration::from_secs(1)).await;
                                copied.set(false);
                            })
                            .detach();
                        }),
                )),
                text(styled),
            ),
        )
        .padding()
        .background(Color::srgb_f32(0.15, 0.15, 0.18))
    }
}

/// Convenience constructor for creating a [`Code`] view inline.
pub fn code(language: impl TryInto<Language, Error: Error>, content: impl Into<Str>) -> Code {
    Code::new(language, content)
}
