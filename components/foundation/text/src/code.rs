//! A fenced code block: a header naming the language, a copy button, and the
//! highlighted source.
//!
//! `Code` is a text-presentation widget, so it lives beside [`Language`],
//! [`DefaultHighlighter`] and [`StyledStr`] rather than in the root crate. That
//! is what lets a component crate claim a fence — install a
//! [`Hook<CodeConfig>`] — without depending on the aggregator.

use alloc::{
    rc::Rc,
    string::{String, ToString},
};
use core::error::Error;
use core::fmt;

#[cfg(target_arch = "wasm32")]
use executor_core::spawn_local;
use waterui_core::gesture::{GestureObserver, TapGesture};
use waterui_core::view::{ConfigurableView, Hook, ViewConfiguration};
use waterui_core::{AnyView, Environment, Metadata, View};
use waterui_graphics::color::Color;
use waterui_layout::{
    background::background,
    padding::{EdgeInsets, Padding},
    spacer,
    stack::{HorizontalAlignment, VStack, hstack},
};
use waterui_str::Str;

use crate::{
    font::{Body, Font},
    highlight::{DefaultHighlighter, Highlighter, Language},
    styled::{Style, StyledStr},
    text,
};

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

/// What runs after a block's text has landed on the clipboard.
///
/// `Code` owns the copy; what to tell the user about it is the caller's — a
/// snackbar, a status line, nothing. The closure is erased here so
/// [`Code::on_copied`] can take a plain generic and [`CodeConfig`] can carry
/// it through a hook that declines the fence.
#[derive(Clone)]
pub struct OnCopied(Rc<dyn Fn(&Environment)>);

impl OnCopied {
    /// Runs the callback against the environment the copy happened in.
    pub fn call(&self, env: &Environment) {
        (self.0)(env);
    }
}

impl fmt::Debug for OnCopied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OnCopied(..)")
    }
}

/// View that renders syntax-highlighted code snippets.
#[derive(Debug, Clone)]
pub struct Code {
    info: Option<Str>,
    language: Language,
    content: Str,
    on_copied: Option<OnCopied>,
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
            on_copied: None,
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

    /// Runs after the block's text has been copied to the clipboard.
    ///
    /// `Code` owns the copy; what to tell the user about it is the caller's —
    /// a snackbar, a status line, nothing. Without one, copying is silent.
    #[must_use]
    pub fn on_copied(mut self, callback: impl Fn(&Environment) + 'static) -> Self {
        self.on_copied = Some(OnCopied(Rc::new(callback)));
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
    /// The caller's feedback for a copy, if it asked for any. Carried so a hook
    /// that declines the fence and renders the default keeps it.
    pub on_copied: Option<OnCopied>,
}

impl ViewConfiguration for CodeConfig {
    type View = Code;

    fn render(self) -> Self::View {
        Code {
            info: self.info,
            language: self.language,
            content: self.content,
            on_copied: self.on_copied,
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
            on_copied: self.on_copied,
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
        on_copied,
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

    // Code block with dark background, left-aligned content.
    let block = VStack::new(
        HorizontalAlignment::Leading,
        8.0,
        (
            hstack((
                text(lang_name)
                    .bold()
                    .color(Color::srgb_f32(0.85, 0.86, 0.9)),
                spacer(),
                copy_button(content_for_copy, on_copied),
            )),
            text(styled),
        ),
    );
    background(
        Padding::new(EdgeInsets::all(14.0), block),
        Color::srgb_f32(0.15, 0.15, 0.18),
    )
}

fn copy_button(content: String, on_copied: Option<OnCopied>) -> impl View {
    Metadata::new(
        text("Copy").color(Color::srgb_f32(0.72, 0.74, 0.8)),
        GestureObserver::new(TapGesture::new(), move |env: Environment| {
            copy_to_clipboard(&content);
            if let Some(on_copied) = &on_copied {
                on_copied.call(&env);
            }
        }),
    )
}

/// Convenience constructor for creating a [`Code`] view inline.
pub fn code(language: impl TryInto<Language, Error: Error>, content: impl Into<Str>) -> Code {
    Code::new(language, content)
}
