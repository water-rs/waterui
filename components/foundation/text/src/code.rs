//! A fenced code block: a header naming the language, a copy button, and the
//! highlighted source.
//!
//! `Code` is a text-presentation widget, so it lives beside [`Language`],
//! [`DefaultHighlighter`] and [`StyledStr`] rather than in the root crate. That
//! is what lets a component crate claim a fence — supply a hook targeting
//! [`CodeConfig`] — without depending on the aggregator.

use alloc::{
    borrow::ToOwned,
    rc::Rc,
    string::{String, ToString},
};
use core::fmt;

use nami::{Computed, Signal, SignalExt, signal::IntoComputed};
use suiteki::Str;
use waterui_color::CurrentColorScheme;
use waterui_core::layout::StretchAxis;
use waterui_core::plugin::Plugin;
use waterui_core::resolve::Resolvable;
use waterui_core::{Environment, Native, NativeView, View};

use crate::{
    highlight::{DefaultHighlighter, Language, highlight_text},
    styled::StyledStr,
};

/// Copies text to the system clipboard.
///
/// `waterkit_clipboard` keeps serving the selection after `set_text`
/// returns — an X11 worker thread holds the data, a forked child holds it
/// on Wayland — so the handle may be scoped to the call.
#[cfg(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_arch = "wasm32"
))]
fn copy_to_clipboard(text: &str) {
    match waterkit_clipboard::Clipboard::new() {
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

/// `waterkit_clipboard` builds only where a system clipboard exists; other
/// targets copy nothing.
#[cfg(not(any(
    target_os = "windows",
    target_os = "linux",
    target_os = "macos",
    target_os = "ios",
    target_os = "android",
    target_arch = "wasm32"
)))]
fn copy_to_clipboard(_text: &str) {}

/// What runs after a block's text has landed on the clipboard.
///
/// `Code` owns the copy; what to tell the user about it is the caller's — a
/// snackbar, a status line, nothing. The closure is erased here so
/// [`Code::on_copied`] can take a plain generic and [`CodeConfig`] can carry
/// it through a hook that declines the fence.
#[derive(Clone)]
pub struct OnCopied(Rc<dyn Fn(&Environment)>);

impl OnCopied {
    /// Creates a new copy-feedback callback.
    pub fn new(callback: impl Fn(&Environment) + 'static) -> Self {
        Self(Rc::new(callback))
    }

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

/// Feedback hook installed by a host window when copying a fenced code block
/// should notify the user (for example via a snackbar).
#[derive(Debug, Clone)]
pub struct CodeCopyFeedback(pub OnCopied);

impl Plugin for CodeCopyFeedback {}

/// A language specification accepted by [`Code::new`] and [`code`].
///
/// Converts infallibly from a typed [`Language`] or any string token (`&str`,
/// `String`, `Str`). Unknown tokens resolve to [`Language::Plaintext`], keep
/// the raw token as [`CodeConfig::info`], and display it in the block header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CodeLanguage {
    language: Language,
    info: Option<Str>,
}

impl CodeLanguage {
    /// Creates a `CodeLanguage` from a typed [`Language`].
    #[must_use]
    pub const fn new(language: Language) -> Self {
        Self {
            language,
            info: None,
        }
    }

    /// Parses a fence or language token into a `CodeLanguage` without failing.
    ///
    /// Extracts the first whitespace-delimited token (matching markdown code
    /// fence info strings). An empty or whitespace-only string resolves to
    /// [`Language::Plaintext`] with no info token; an unrecognized token
    /// resolves to [`Language::Plaintext`] while keeping the original token as
    /// [`info`](Self::info).
    #[must_use]
    pub fn parse(token: &str) -> Self {
        let Some(first) = token.split_whitespace().next() else {
            return Self::new(Language::Plaintext);
        };
        let language = first.parse::<Language>().unwrap_or(Language::Plaintext);
        Self {
            language,
            info: Some(Str::from(first.to_owned())),
        }
    }

    fn parse_owned(raw: Str) -> Self {
        let Some(first) = raw.split_whitespace().next() else {
            return Self::new(Language::Plaintext);
        };
        let language = first.parse::<Language>().unwrap_or(Language::Plaintext);
        let info = if first.len() == raw.len() {
            raw
        } else {
            Str::from(first.to_owned())
        };
        Self {
            language,
            info: Some(info),
        }
    }

    /// The resolved highlighting [`Language`].
    #[must_use]
    pub const fn language(&self) -> &Language {
        &self.language
    }

    /// The original token preserved from string input, if any.
    #[must_use]
    pub const fn info(&self) -> Option<&Str> {
        self.info.as_ref()
    }
}

impl From<Language> for CodeLanguage {
    fn from(language: Language) -> Self {
        Self::new(language)
    }
}

impl From<&str> for CodeLanguage {
    fn from(token: &str) -> Self {
        Self::parse(token)
    }
}

impl From<String> for CodeLanguage {
    fn from(token: String) -> Self {
        Self::parse_owned(Str::from(token))
    }
}

impl From<Str> for CodeLanguage {
    fn from(token: Str) -> Self {
        Self::parse_owned(token)
    }
}

impl From<&Str> for CodeLanguage {
    fn from(token: &Str) -> Self {
        Self::parse_owned(token.clone())
    }
}

/// View that renders syntax-highlighted code snippets.
#[derive(Debug, Clone)]
pub struct Code {
    info: Option<Str>,
    language: Language,
    content: Computed<Str>,
    on_copied: Option<OnCopied>,
}

impl Code {
    /// Creates a new `Code` view for the provided language and content.
    ///
    /// Accepts a typed [`Language`] or any string token via [`CodeLanguage`].
    /// Unrecognized tokens render as [`Language::Plaintext`], preserve the
    /// original token as [`CodeConfig::info`], and display it in the header.
    ///
    /// `content` accepts static strings as well as reactive signals
    /// (`Binding<Str>`, `Computed<Str>`, etc.) and re-highlights in place when
    /// the signal updates.
    pub fn new(language: impl Into<CodeLanguage>, content: impl IntoComputed<Str>) -> Self {
        let CodeLanguage { language, info } = language.into();
        Self {
            info,
            language,
            content: content.into_computed(),
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
/// An application claims a fence with a hook targeting `CodeConfig` over the
/// views that hold it; a hook that does not recognise [`info`](Self::info)
/// declines, and the ordinary code block is drawn.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct CodeConfig {
    /// The fence's info token as written (`mermaid`, `rust`, …), if it had one.
    info: Option<Str>,
    /// The language that token resolved to; [`Language::Plaintext`] when it
    /// resolved to nothing.
    language: Language,
    /// The block's reactive source text, verbatim.
    content: Computed<Str>,
    /// The caller's feedback for a copy, if it asked for any. Carried so a hook
    /// that declines the fence and renders the default keeps it.
    on_copied: Option<OnCopied>,
}

impl CodeConfig {
    /// The fence's info token as written (`mermaid`, `rust`, …), if it had one.
    #[must_use]
    pub const fn info(&self) -> Option<&Str> {
        self.info.as_ref()
    }

    /// The language that token resolved to; [`Language::Plaintext`] when it
    /// resolved to nothing.
    #[must_use]
    pub const fn language(&self) -> &Language {
        &self.language
    }

    /// The block's reactive source text, verbatim.
    #[must_use]
    pub const fn content(&self) -> &Computed<Str> {
        &self.content
    }

    /// The caller's feedback for a copy, if it asked for any. Carried so a hook
    /// that declines the fence and renders the default keeps it.
    #[must_use]
    pub const fn on_copied(&self) -> Option<&OnCopied> {
        self.on_copied.as_ref()
    }

    /// The header caption naming this block's language or unrecognized fence token.
    #[must_use]
    pub fn header_label(&self) -> String {
        header_label(&self.language, self.info.as_deref())
    }

    /// Reactive syntax-highlighted text following the installed color scheme.
    #[must_use]
    pub fn highlighted(&self, env: &Environment) -> Computed<StyledStr> {
        let language = self.language.clone();
        self.content
            .zip(&CurrentColorScheme.resolve(env))
            .map(move |(content, scheme)| {
                highlight_text(
                    language.clone(),
                    &content,
                    &mut DefaultHighlighter::new(scheme),
                )
            })
            .computed()
    }

    /// Copies the block's current source text to the system clipboard and runs
    /// any registered [`OnCopied`] callback.
    pub fn copy_to_clipboard(&self, env: &Environment) {
        let text = self.content.snapshot();
        copy_to_clipboard(&text);
        if let Some(on_copied) = &self.on_copied {
            on_copied.call(env);
        }
    }
}

impl From<CodeConfig> for Code {
    fn from(value: CodeConfig) -> Self {
        Self {
            info: value.info,
            language: value.language,
            content: value.content,
            on_copied: value.on_copied,
        }
    }
}

impl Code {
    /// Returns the configuration for this code block.
    #[must_use]
    pub fn config(self) -> CodeConfig {
        CodeConfig {
            info: self.info,
            language: self.language,
            content: self.content,
            on_copied: self.on_copied,
        }
    }
}

impl NativeView for CodeConfig {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Horizontal
    }
}

impl View for Code {
    fn body(self, _env: &Environment) -> impl View {
        Native::new(self.config())
    }
}

fn header_label(language: &Language, info: Option<&str>) -> String {
    match (language, info) {
        (Language::Plaintext, Some(token))
            if !token.is_empty() && token.parse::<Language>().is_err() =>
        {
            token.to_owned()
        }
        _ => language.to_string(),
    }
}

/// Convenience constructor for creating a [`Code`] view inline.
pub fn code(language: impl Into<CodeLanguage>, content: impl IntoComputed<Str>) -> Code {
    Code::new(language, content)
}

#[cfg(test)]
mod tests {
    use nami::Binding;

    use super::*;

    #[test]
    fn code_content_accepts_reactive_signal_and_tracks_updates() {
        let source = Binding::new(Str::from("fn alpha() {}"));
        let config = Code::new(Language::Rust, source.clone()).config();
        assert_eq!(config.content().snapshot().as_str(), "fn alpha() {}");

        source.set(Str::from("fn beta() {}"));
        assert_eq!(config.content().snapshot().as_str(), "fn beta() {}");
    }

    #[test]
    fn unknown_language_renders_as_plaintext_and_preserves_token_for_header() {
        let block = Code::new("tsx", "export const A = () => <div />;");
        let config = block.config();
        assert_eq!(config.language(), &Language::Plaintext);
        assert_eq!(config.info().map(Str::as_str), Some("tsx"));
        assert_eq!(
            header_label(config.language(), config.info().map(Str::as_str)),
            "tsx"
        );
    }

    #[test]
    fn known_language_token_resolves_and_uses_canonical_header() {
        let block = Code::new("rust", "fn main() {}");
        let config = block.config();
        assert_eq!(config.language(), &Language::Rust);
        assert_eq!(config.info().map(Str::as_str), Some("rust"));
        assert_eq!(
            header_label(config.language(), config.info().map(Str::as_str)),
            "Rust"
        );
    }

    #[test]
    fn typed_language_and_empty_token_work_without_info() {
        let typed = Code::new(Language::Python, "pass").config();
        assert_eq!(typed.language(), &Language::Python);
        assert_eq!(typed.info(), None);
        assert_eq!(
            header_label(typed.language(), typed.info().map(Str::as_str)),
            "Python"
        );

        let empty = Code::new("   ", "plain").config();
        assert_eq!(empty.language(), &Language::Plaintext);
        assert_eq!(empty.info(), None);
        assert_eq!(
            header_label(empty.language(), empty.info().map(Str::as_str)),
            "Plaintext"
        );
    }

    #[test]
    fn fence_metadata_extracts_first_token_matching_markdown() {
        let mermaid = Code::new(Str::from("mermaid title=flow"), "graph TD; A-->B").config();
        assert_eq!(mermaid.language(), &Language::Plaintext);
        assert_eq!(mermaid.info().map(Str::as_str), Some("mermaid"));
        assert_eq!(
            header_label(mermaid.language(), mermaid.info().map(Str::as_str)),
            "mermaid"
        );
    }
}
