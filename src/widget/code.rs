use core::error::Error;
#[cfg(feature = "snackbar")]
use waterui_core::State;
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, View};
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

#[cfg(target_arch = "wasm32")]
use executor_core::spawn_local;

use crate::ViewExt;
#[cfg(feature = "snackbar")]
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
    info: Option<Str>,
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
            info: None,
            language: language.try_into().expect("Invalid language"),
            content: content.into(),
        }
    }

    /// Records the fence's info token as the author wrote it.
    ///
    /// Markdown calls this so a realization can claim a token no [`Language`]
    /// answers to — `mermaid`, `plantuml`, `vega` — which would otherwise be
    /// indistinguishable from a fence carrying no info string at all.
    #[must_use]
    pub fn info(mut self, info: impl Into<Str>) -> Self {
        self.info = Some(info.into());
        self
    }
}

/// Everything a fenced code block is, before anything decides how to show it.
///
/// An application claims a fence by installing a [`Hook<CodeConfig>`] on its
/// environment; a hook that does not recognise [`info`](Self::info) calls
/// [`ViewConfiguration::render`] to get the ordinary code block back.
#[derive(Debug, Clone)]
pub struct CodeConfig {
    /// The fence's info token as written (`mermaid`, `rust`, …), if it had one.
    pub info: Option<Str>,
    /// The language that token resolved to; [`Language::Plaintext`] when it
    /// resolved to nothing.
    pub language: Language,
    /// The block's text, verbatim.
    pub content: Str,
}

impl ViewConfiguration for CodeConfig {
    type View = Code;

    fn render(self) -> Self::View {
        Code {
            info: self.info,
            language: self.language,
            content: self.content,
        }
    }
}

impl ConfigurableView for Code {
    type Config = CodeConfig;

    fn config(self) -> Self::Config {
        CodeConfig {
            info: self.info,
            language: self.language,
            content: self.content,
        }
    }
}

impl View for Code {
    fn body(self, env: &Environment) -> impl View {
        let config = self.config();
        // `Hook::from` removes the hook from the environment it hands to the
        // closure, so a realization that does not recognise the info token can
        // call `config.render()` and get this default rendering back without
        // recursing into itself.
        if let Some(hook) = env.get::<Hook<CodeConfig>>() {
            AnyView::new(hook.apply(env, config))
        } else {
            AnyView::new(default_rendering(config))
        }
    }
}

/// What a fence looks like when nothing claims it: a header naming the
/// language, a copy button, and the highlighted source.
fn default_rendering(config: CodeConfig) -> impl View {
    let CodeConfig {
        info: _,
        language,
        content,
    } = config;
    let lang_name = language.to_string();
    let content_for_copy = content.to_string();
    let mut highlighter = DefaultHighlighter::new();
    let chunks = highlighter.highlight(language, &content);

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
                copy_button(content_for_copy),
            )),
            text(styled),
        ),
    )
    .padding()
    .background(Color::srgb_f32(0.15, 0.15, 0.18))
}

#[cfg(feature = "snackbar")]
fn copy_button(content: String) -> impl View {
    text("Copy")
        .foreground(Color::srgb_f32(0.72, 0.74, 0.8))
        .on_tap(move |State(snackbar): State<SnackbarManager>| {
            copy_to_clipboard(&content);
            snackbar.show(Snackbar::new("Copied to clipboard"));
        })
}

#[cfg(not(feature = "snackbar"))]
fn copy_button(content: String) -> impl View {
    text("Copy")
        .foreground(Color::srgb_f32(0.72, 0.74, 0.8))
        .on_tap(move || copy_to_clipboard(&content))
}

/// Convenience constructor for creating a [`Code`] view inline.
pub fn code(language: impl TryInto<Language, Error: Error>, content: impl Into<Str>) -> Code {
    Code::new(language, content)
}
