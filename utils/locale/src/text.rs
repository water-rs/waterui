//! `LocalizedText` view for i18n support.

use nami::{Binding, SignalExt};
use waterui_core::{Environment, View, dynamic::watch};
use waterui_text::{
    Text,
    font::{Body, Caption, Font, Footnote, Headline, Subheadline, Title},
    styled::StyledStr,
};

use crate::locale::Locale;
use crate::system::runtime_locale_binding;

/// A localized text view that renders based on the current locale.
///
/// This is the return type of the `text!` macro. It wraps a function
/// that takes a locale and returns a `Text` view.
///
/// Text reacts to locale changes from the shared regional runtime automatically.
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
        font: impl Into<Font>,
    ) -> LocalizedText<F, impl Fn(StyledStr) -> StyledStr + Clone + 'static>
    where
        T: Clone + 'static,
    {
        let font = font.into();
        let prev_transform = self.transform;
        LocalizedText {
            text_fn: self.text_fn,
            transform: move |t| prev_transform(t).font(font.clone()),
        }
    }
}

macro_rules! impl_font {
    ($doc:expr,$name:ident,$font:expr) => {
        impl<F, T> LocalizedText<F, T>
        where
            F: Fn(&Locale) -> Text,
            T: Fn(StyledStr) -> StyledStr,
        {
            #[doc = $doc]
            #[must_use]
            pub fn $name(
                self,
            ) -> LocalizedText<F, impl Fn(StyledStr) -> StyledStr + Clone + 'static>
            where
                T: Clone + 'static,
            {
                LocalizedText::font(self, $font)
            }
        }
    };
}

impl_font!("Set font to title.", title, Title);
impl_font!("Set font to headline.", headline, Headline);
impl_font!("Set font to sub_headline.", sub_headline, Subheadline);
impl_font!("Set font to body.", body, Body);
impl_font!("Set font to caption.", caption, Caption);
impl_font!("Set font to footnote.", footnote, Footnote);

impl<F, T> View for LocalizedText<F, T>
where
    F: Fn(&Locale) -> Text + Clone + 'static,
    T: Fn(StyledStr) -> StyledStr + Clone + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        let locale = resolve_locale_binding(env);

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

fn resolve_locale_binding(env: &Environment) -> Binding<Locale> {
    // Respect explicit per-view RegionalContext in the environment first.
    if let Some(context) = env.get::<crate::regional::RegionalContext>().cloned() {
        return Binding::container(context.locale().clone());
    }

    // Respect explicit per-view Locale in the environment first.
    if let Some(locale) = env.get::<Locale>().cloned() {
        return Binding::container(locale);
    }

    // Otherwise, track the shared runtime locale.
    runtime_locale_binding()
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

#[cfg(test)]
mod tests {
    use waterui_core::Environment;

    use super::resolve_locale_binding;
    use crate::locale::{Locale, locales};

    #[test]
    fn resolve_locale_signal_uses_static_locale_from_environment() {
        let mut env = Environment::new();
        env.insert(locales::EN_GB);

        let locale = resolve_locale_binding(&env).get();
        assert_eq!(locale.language.as_str(), "en");
        assert_eq!(locale.region.as_ref().map(|r| r.as_str()), Some("GB"));
    }

    #[test]
    fn resolve_locale_signal_defaults_to_en_us() {
        let env = Environment::new();
        let locale: Locale = resolve_locale_binding(&env).get();
        assert!(!locale.language.as_str().is_empty());
    }
}
