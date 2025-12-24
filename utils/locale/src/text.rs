//! `LocalizedText` view for i18n support.

use nami::{Computed, SignalExt};
use waterui_core::{Environment, View, dynamic::watch};
use waterui_text::{Text, font::Font, styled::StyledStr};

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
pub struct LocalizedText<F, T = fn(StyledStr) -> StyledStr>
where
    F: Fn(&Locale) -> Text,
    T: Fn(StyledStr) -> StyledStr,
{
    /// Function that generates text based on locale
    text_fn: F,
    /// Transform to apply to the text (for styling)
    transform: T,
}

impl<F> LocalizedText<F, fn(StyledStr) -> StyledStr>
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
    T: Fn(StyledStr) -> StyledStr,
{
    /// Sets the font size.
    #[must_use]
    pub fn size(
        self,
        size: f32,
    ) -> LocalizedText<F, impl Fn(StyledStr) -> StyledStr + Clone + 'static>
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
    pub fn bold(self) -> LocalizedText<F, impl Fn(StyledStr) -> StyledStr + Clone + 'static>
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
    pub fn italic(self) -> LocalizedText<F, impl Fn(StyledStr) -> StyledStr + Clone + 'static>
    where
        T: Clone + 'static,
    {
        let prev_transform = self.transform;
        LocalizedText {
            text_fn: self.text_fn,
            transform: move |t| prev_transform(t).italic(true),
        }
    }

    /// Sets the font.
    #[must_use]
    pub fn font(
        self,
        font: Font,
    ) -> LocalizedText<F, impl Fn(StyledStr) -> StyledStr + Clone + 'static>
    where
        T: Clone + 'static,
    {
        let prev_transform = self.transform;
        LocalizedText {
            text_fn: self.text_fn,
            transform: move |t| prev_transform(t).font(font.clone()),
        }
    }
}

impl<F, T> View for LocalizedText<F, T>
where
    F: Fn(&Locale) -> Text + Clone + 'static,
    T: Fn(StyledStr) -> StyledStr + Clone + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        // Get locale - either reactive Computed<Locale> or static Locale wrapped in constant
        let locale = env.get::<Computed<Locale>>().cloned().unwrap_or_else(|| {
            // Fallback to default locale if none found
            Computed::constant(locales::EN_US)
        });

        // Map locale to styled content reactively
        let text_fn = self.text_fn;
        let transform = self.transform;

        watch(locale, move |locale| {
            let text = text_fn(&locale);
            let styled = text.content();
            let transform = transform.clone();
            let styled = styled.map(move |styled| transform(styled));
            Text::new(styled)
        })
    }
}

impl<F, T> core::fmt::Debug for LocalizedText<F, T>
where
    F: Fn(&Locale) -> Text,
    T: Fn(StyledStr) -> StyledStr,
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LocalizedText").finish_non_exhaustive()
    }
}
