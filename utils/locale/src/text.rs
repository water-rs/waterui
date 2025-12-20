//! `LocalizedText` view for i18n support.

use nami::{Computed, Signal, SignalExt};
use waterui_core::{Environment, View};
use waterui_text::Text;
use waterui_text::styled::StyledStr;

use crate::locale::{Locale, locales};

/// A localized text view that renders based on the current locale.
///
/// This is the return type of the `text!` macro. It wraps a function
/// that takes a locale and returns a `Text` view.
///
/// When the environment contains a `Computed<Locale>`, the text will
/// automatically update when the locale changes. This makes `LocalizedText`
/// reactive without requiring explicit `Dynamic` or `watch` calls.
///
/// # Examples
///
/// ```rust,ignore
/// use waterui_locale::text;
///
/// fn my_view(count: i32) -> impl View {
///     // text! macro returns LocalizedText
///     text!("I have {#count} apple").size(24.0)
/// }
/// ```
pub struct LocalizedText<F, T = fn(Text) -> Text>
where
    F: Fn(&Locale) -> Text,
    T: Fn(Text) -> Text,
{
    /// Function that generates text based on locale
    text_fn: F,
    /// Transform to apply to the text (for styling)
    transform: T,
}

impl<F> LocalizedText<F, fn(Text) -> Text>
where
    F: Fn(&Locale) -> Text,
{
    /// Create a new LocalizedText with no transform
    pub fn new(text_fn: F) -> Self {
        Self {
            text_fn,
            transform: |t| t,
        }
    }
}

impl<F, T> LocalizedText<F, T>
where
    F: Fn(&Locale) -> Text,
    T: Fn(Text) -> Text,
{
    /// Sets the font size.
    #[must_use]
    pub fn size(self, size: f64) -> LocalizedText<F, impl Fn(Text) -> Text + Clone + 'static>
    where
        T: Clone + 'static,
    {
        let prev_transform = self.transform;
        LocalizedText {
            text_fn: self.text_fn,
            transform: move |t| prev_transform(t).size(size),
        }
    }

    /// Makes the text bold.
    #[must_use]
    pub fn bold(self) -> LocalizedText<F, impl Fn(Text) -> Text + Clone + 'static>
    where
        T: Clone + 'static,
    {
        let prev_transform = self.transform;
        LocalizedText {
            text_fn: self.text_fn,
            transform: move |t| prev_transform(t).bold(),
        }
    }

    /// Makes the text italic.
    #[must_use]
    pub fn italic(self) -> LocalizedText<F, impl Fn(Text) -> Text + Clone + 'static>
    where
        T: Clone + 'static,
    {
        let prev_transform = self.transform;
        LocalizedText {
            text_fn: self.text_fn,
            transform: move |t| prev_transform(t).italic(true),
        }
    }
}

impl<F, T> View for LocalizedText<F, T>
where
    F: Fn(&Locale) -> Text + Clone + 'static,
    T: Fn(Text) -> Text + Clone + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        // Try to get Computed<Locale> for reactive locale changes
        if let Some(locale_computed) = env.get::<Computed<Locale>>() {
            // Create a reactive text that updates when locale changes
            let locale_signal = locale_computed.clone();
            let text_fn = self.text_fn;
            let transform = self.transform;

            // Map the locale signal to styled string
            let content: Computed<StyledStr> = locale_signal
                .map(move |locale| {
                    let text = text_fn(&locale);
                    let transformed = transform(text);
                    // Extract the styled string from the text using public content() method
                    transformed.content().get()
                })
                .computed();

            Text::new(content)
        } else {
            // Fallback: extract static Locale from environment
            let locale = env
                .get::<Locale>()
                .cloned()
                .unwrap_or_else(|| locales::EN.clone());

            // Call the inner function with the locale and apply transform
            let text = (self.text_fn)(&locale);
            (self.transform)(text)
        }
    }
}

impl<F, T> core::fmt::Debug for LocalizedText<F, T>
where
    F: Fn(&Locale) -> Text,
    T: Fn(Text) -> Text,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalizedText").finish_non_exhaustive()
    }
}
