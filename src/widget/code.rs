use core::error::Error;
use waterui_core::{State, View};
use waterui_graphics::color::Color;
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

use crate::ViewExt;
use crate::snackbar::{Snackbar, SnackbarManager};

/// Copies text to the system clipboard.
#[cfg(all(
    not(target_os = "android"),
    not(target_arch = "wasm32"),
    not(target_os = "espidf")
))]
fn copy_to_clipboard(text: &str) {
    match arboard::Clipboard::new() {
        Ok(mut clipboard) => {
            if let Err(error) = clipboard.set_text(text) {
                tracing::error!(%error, "Failed to copy to clipboard");
            }
        }
        Err(error) => {
            tracing::error!(%error, "Failed to access clipboard");
        }
    }
}

/// Embedded targets have no system clipboard; copy is a no-op.
#[cfg(target_os = "espidf")]
fn copy_to_clipboard(_text: &str) {}

/// Copies text to the Android clipboard.
#[cfg(target_os = "android")]
fn copy_to_clipboard(text: &str) {
    if let Err(error) = android_clipboard::set_text(text.to_string()) {
        tracing::error!(%error, "Failed to copy to clipboard");
    }
}

/// Copies text to the browser clipboard.
#[cfg(target_arch = "wasm32")]
fn copy_to_clipboard(text: &str) {
    let clipboard = web_sys::window()
        .expect("browser window is unavailable for clipboard access")
        .navigator()
        .clipboard();
    let text = text.to_string();

    spawn_local(async move {
        let promise = clipboard.write_text(&text);
        if let Err(error) = wasm_bindgen_futures::JsFuture::from(promise).await {
            tracing::error!(?error, "Failed to write browser clipboard text");
        }
    })
    .detach();
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

        // Code block with dark background, left-aligned content. Copy
        // feedback is presented through the window's SnackbarManager (a
        // semantic object owned by the runtime) instead of view-local state.
        VStack::new(
            HorizontalAlignment::Leading,
            8.0,
            (
                hstack((
                    text(lang_name)
                        .bold()
                        .foreground(Color::srgb_f32(0.85, 0.86, 0.9)),
                    spacer(),
                    text("Copy")
                        .foreground(Color::srgb_f32(0.72, 0.74, 0.8))
                        .on_tap(move |State(snackbar): State<SnackbarManager>| {
                            copy_to_clipboard(&content_for_copy);
                            snackbar.show(Snackbar::new("Copied to clipboard"));
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
