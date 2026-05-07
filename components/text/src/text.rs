use alloc::{
    rc::Rc,
    string::{String, ToString},
};
use core::cell::RefCell;
use core::fmt;
use core::ops::{Add, AddAssign};
use fmt::Display;

use nami::signal::{IntoComputed, IntoSignal};
use nami::watcher::{BoxWatcherGuard, Context, WatcherGuard};
use nami::{Computed, Signal, SignalExt};
use waterui_core::configurable;
use waterui_core::layout::HorizontalAlignment;
use waterui_core::{Environment, View};
use waterui_graphics::color::Color;
use waterui_locale::{Locale, TranslationCatalog, locale_binding};
use waterui_str::Str;

use crate::font::FontWeight;
use crate::locale::Formatter;
use crate::{font::Font, styled::StyledStr};

type ConfigResolver = dyn Fn(&Environment) -> Computed<TextConfig>;

/// Native text payload consumed by platform backends.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct TextConfig {
    /// The rich text content to be displayed.
    pub content: Computed<StyledStr>,
    /// Paragraph alignment for multiline layout.
    pub paragraph_alignment: Computed<HorizontalAlignment>,
}

impl TextConfig {
    /// Creates a raw text config from styled content.
    #[must_use]
    pub fn new(content: impl IntoComputed<StyledStr>) -> Self {
        Self {
            content: content.into_signal().map(StyledStr::from).computed(),
            paragraph_alignment: Computed::constant(HorizontalAlignment::Leading),
        }
    }
}

configurable!(
    /// Raw styled text view.
    RawText,
    TextConfig
);

#[derive(Clone)]
enum TextKind {
    Raw(TextConfig),
    Signal { resolver: Rc<ConfigResolver> },
}

/// Semantic text view with integrated localization support.
#[must_use]
#[derive(Clone)]
pub struct Text(TextKind);

impl fmt::Debug for Text {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Text").finish_non_exhaustive()
    }
}

impl core::cmp::PartialEq for Text {
    fn eq(&self, _other: &Self) -> bool {
        false
    }
}

impl core::cmp::PartialOrd for Text {
    fn partial_cmp(&self, _other: &Self) -> Option<core::cmp::Ordering> {
        None
    }
}

/// Conversion trait for semantic text construction.
pub trait IntoText {
    /// Convert a value into semantic text.
    fn into_text(self) -> Text;
}

impl IntoText for Text {
    fn into_text(self) -> Text {
        self
    }
}

impl IntoText for &'static str {
    fn into_text(self) -> Text {
        Text::localized(self)
    }
}

impl IntoText for String {
    fn into_text(self) -> Text {
        Text::verbatim(self)
    }
}

impl IntoText for Str {
    fn into_text(self) -> Text {
        Text::verbatim(self)
    }
}

impl IntoText for StyledStr {
    fn into_text(self) -> Text {
        Text::computed(self)
    }
}

impl<T> IntoText for Computed<T>
where
    T: IntoText + Clone + 'static,
{
    fn into_text(self) -> Text {
        Text::from_signal(self)
    }
}

impl From<&'static str> for Text {
    fn from(value: &'static str) -> Self {
        value.into_text()
    }
}

impl From<String> for Text {
    fn from(value: String) -> Self {
        value.into_text()
    }
}

impl From<Str> for Text {
    fn from(value: Str) -> Self {
        value.into_text()
    }
}

impl From<StyledStr> for Text {
    fn from(value: StyledStr) -> Self {
        value.into_text()
    }
}

impl<T> From<Computed<T>> for Text
where
    T: IntoText + Clone + 'static,
{
    fn from(value: Computed<T>) -> Self {
        value.into_text()
    }
}

impl<T> Add<T> for Text
where
    T: IntoText,
{
    type Output = Self;

    fn add(self, rhs: T) -> Self::Output {
        let rhs = rhs.into_text();
        Self::from_config_signal(move |env| {
            self.resolve_signal(env)
                .zip(&rhs.resolve_signal(env))
                .map(|(lhs, rhs)| TextConfig {
                    content: lhs.content.zip(&rhs.content).map(|(a, b)| a + b).computed(),
                    paragraph_alignment: lhs.paragraph_alignment,
                })
                .computed()
        })
    }
}

impl<T> AddAssign<T> for Text
where
    T: IntoText,
{
    fn add_assign(&mut self, rhs: T) {
        *self = self.clone() + rhs;
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::verbatim("")
    }
}

impl Text {
    /// Creates semantic text using the default conversion rules.
    pub fn new(content: impl IntoText) -> Self {
        content.into_text()
    }

    /// Creates raw text from a computed styled string.
    pub fn computed(content: impl IntoComputed<StyledStr>) -> Self {
        Self(TextKind::Raw(TextConfig::new(content)))
    }

    /// Creates verbatim text that never consults the translation catalog.
    pub fn verbatim(content: impl Into<Str>) -> Self {
        Self::computed(StyledStr::plain(content.into()))
    }

    /// Creates localized text that resolves through the runtime translation catalog.
    pub fn localized(key: &'static str) -> Self {
        Self::localized_with(move |env, locale| {
            let translated = env
                .get::<TranslationCatalog>()
                .and_then(|catalog| catalog.lookup_text(locale, key))
                .unwrap_or_else(|| Str::from_static(key));
            TextConfig::new(StyledStr::plain(translated))
        })
    }

    /// Creates text from a locale-aware config resolver.
    pub fn localized_with(
        resolver: impl Fn(&Environment, &Locale) -> TextConfig + 'static,
    ) -> Self {
        let resolver = Rc::new(resolver);
        Self::from_config_signal(move |env| {
            let env = env.clone();
            let resolver = resolver.clone();
            locale_binding(&env)
                .map(move |locale| resolver(&env, &locale))
                .computed()
        })
    }

    fn from_config_signal(
        resolver: impl Fn(&Environment) -> Computed<TextConfig> + 'static,
    ) -> Self {
        Self(TextKind::Signal {
            resolver: Rc::new(resolver),
        })
    }

    fn from_signal<S, T>(source: S) -> Self
    where
        S: Signal<Output = T> + Clone + 'static,
        T: IntoText + Clone + 'static,
    {
        Self::from_config_signal(move |env| {
            let env = env.clone();
            source
                .clone()
                .zip(&locale_binding(&env))
                .map(move |(value, _locale)| value.into_text().resolve(&env))
                .computed()
        })
    }

    fn resolve_signal(&self, env: &Environment) -> Computed<TextConfig> {
        match &self.0 {
            TextKind::Raw(config) => Computed::constant(config.clone()),
            TextKind::Signal { resolver } => resolver(env),
        }
    }

    /// Resolves semantic text into a raw config using the environment's effective locale.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> TextConfig {
        self.resolve_signal(env).get()
    }

    /// Converts text into an FFI-ready raw config without an environment.
    ///
    /// # Panics
    ///
    /// Panics when called on localized text because localization must be resolved
    /// from a concrete environment in the component `body(env)` path.
    #[must_use]
    pub fn into_config_without_env(self) -> TextConfig {
        match self.0 {
            TextKind::Raw(config) => config,
            TextKind::Signal { .. } => panic!(
                "Environment-dependent Text cannot cross FFI directly; resolve it in the component body with Text::resolve(env) before converting to native config"
            ),
        }
    }

    /// Creates text from any displayable signal.
    #[allow(clippy::needless_pass_by_value)]
    pub fn display<T: Display>(source: impl Signal<Output = T> + 'static) -> Self {
        Self::computed(source.map(|value| value.to_string()))
    }

    /// Creates locale-formatted text.
    pub fn format<T>(value: impl IntoComputed<T>, formatter: impl Formatter<T> + 'static) -> Self {
        Self::computed(
            value
                .into_signal()
                .map(move |value| formatter.format(&value)),
        )
    }

    /// Returns the computed content for raw text.
    ///
    /// # Panics
    ///
    /// Panics for localized text because the resolved content depends on the environment.
    #[must_use]
    pub fn content(&self) -> Computed<StyledStr> {
        match &self.0 {
            TextKind::Raw(config) => config.content.clone(),
            TextKind::Signal { .. } => {
                panic!(
                    "Text::content() is only available for raw text; environment-dependent text must be resolved with Text::resolve(env)"
                )
            }
        }
    }

    /// Returns the paragraph alignment for raw text.
    ///
    /// # Panics
    ///
    /// Panics for localized text because the resolved configuration depends on the environment.
    #[must_use]
    pub fn paragraph_alignment(&self) -> Computed<HorizontalAlignment> {
        match &self.0 {
            TextKind::Raw(config) => config.paragraph_alignment.clone(),
            TextKind::Signal { .. } => {
                panic!(
                    "Text::paragraph_alignment() is only available for raw text; environment-dependent text must be resolved with Text::resolve(env)"
                )
            }
        }
    }

    fn map_config(self, f: impl Fn(TextConfig) -> TextConfig + Clone + 'static) -> Self {
        match self.0 {
            TextKind::Raw(config) => Self(TextKind::Raw(f(config))),
            TextKind::Signal { resolver } => {
                Self::from_config_signal(move |env| resolver(env).map(f.clone()).computed())
            }
        }
    }

    /// Sets the font for this text component.
    pub fn font(self, font: impl IntoSignal<Font> + 'static) -> Self {
        let font = font.into_signal();
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&font)
                .map(|(content, font)| content.font(font))
                .computed();
            config
        })
    }

    /// Sets the font size.
    pub fn size(self, size: impl IntoSignal<f64> + 'static) -> Self {
        #[allow(clippy::cast_possible_truncation)]
        let size = size.into_signal().map(|s| s as f32);
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&size)
                .map(|(content, size)| content.size(size))
                .computed();
            config
        })
    }

    /// Sets the font weight.
    pub fn weight(self, weight: impl IntoSignal<FontWeight> + 'static) -> Self {
        let weight = weight.into_signal();
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&weight)
                .map(|(content, weight)| content.weight(weight))
                .computed();
            config
        })
    }

    /// Applies an underline to the text.
    pub fn underline(self, underline: impl IntoSignal<bool> + 'static) -> Self {
        let underline = underline.into_signal();
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&underline)
                .map(|(content, underline)| content.underline(underline))
                .computed();
            config
        })
    }

    /// Sets the text color.
    ///
    /// Accepts any [`IntoSignal<Color>`] source: a static `Color` literal,
    /// `Computed<Color>`, `Binding<Color>`, or any other `Signal<Output =
    /// Color>`. The styled-content pipeline subscribes to the signal so a
    /// changing binding re-emits the colored content automatically.
    pub fn color(self, color: impl IntoSignal<Color> + 'static) -> Self {
        let color = color.into_signal();
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&color)
                .map(|(content, color)| content.foreground(color))
                .computed();
            config
        })
    }

    /// Sets the background color for the text.
    pub fn background_color(self, color: impl IntoSignal<Color> + 'static) -> Self {
        let color = color.into_signal();
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&color)
                .map(|(content, color)| content.background_color(color))
                .computed();
            config
        })
    }

    /// Sets the paragraph alignment for multiline text layout.
    pub fn text_align(self, alignment: impl IntoSignal<HorizontalAlignment> + 'static) -> Self {
        let alignment = alignment.into_signal().computed();
        self.map_config(move |mut config| {
            config.paragraph_alignment = alignment.clone();
            config
        })
    }

    /// Sets the font to bold.
    pub fn bold(self) -> Self {
        self.weight(FontWeight::Bold)
    }

    /// Sets the italic style.
    pub fn italic(self, is_italic: impl Signal<Output = bool> + 'static) -> Self {
        self.map_config(move |mut config| {
            config.content = config
                .content
                .zip(&is_italic)
                .map(|(content, is_italic)| content.italic(is_italic))
                .computed();
            config
        })
    }
}

#[derive(Clone)]
struct FlattenSignal<S> {
    nested: S,
}

struct FlattenSignalWatchGuard<G>
where
    G: WatcherGuard,
{
    _outer: G,
    _inner: Rc<RefCell<Option<BoxWatcherGuard>>>,
}

impl<G> WatcherGuard for FlattenSignalWatchGuard<G> where G: WatcherGuard {}

impl<S, T> Signal for FlattenSignal<S>
where
    S: Signal<Output = Computed<T>> + Clone + 'static,
    T: Clone + 'static,
{
    type Output = T;
    type Guard = FlattenSignalWatchGuard<S::Guard>;

    fn get(&self) -> Self::Output {
        self.nested.get().get()
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let watcher = Rc::new(watcher);
        let inner = Rc::new(RefCell::new(None));

        let initial = self.nested.get();
        *inner.borrow_mut() = Some(initial.watch({
            let watcher = watcher.clone();
            move |ctx| watcher(ctx)
        }));

        let outer = self.nested.watch({
            let watcher = watcher;
            let inner = inner.clone();
            move |ctx: Context<Computed<T>>| {
                let next = ctx.value().clone();
                *inner.borrow_mut() = Some(next.watch({
                    let watcher = watcher.clone();
                    move |ctx| watcher(ctx)
                }));
                watcher(Context::new(next.get(), ctx.metadata().clone()));
            }
        });

        FlattenSignalWatchGuard {
            _outer: outer,
            _inner: inner,
        }
    }
}

fn flatten_signal<S, T>(nested: S) -> Computed<T>
where
    S: Signal<Output = Computed<T>> + Clone + 'static,
    T: Clone + 'static,
{
    Computed::new(FlattenSignal { nested })
}

macro_rules! impl_text_font {
    ($(($name:ident, $value:expr)),+) => {
        $(
            impl Text {
                #[doc = concat!("Applies the `", stringify!($name), "` text style preset.")]
                pub fn $name(self) -> Self {
                    self.font($value)
                }
            }
        )+
    };
}

impl_text_font!(
    (body, crate::font::Body),
    (title, crate::font::Title),
    (headline, crate::font::Headline),
    (sub_headline, crate::font::Subheadline),
    (caption, crate::font::Caption),
    (footnote, crate::font::Footnote)
);

/// Creates semantic text using the default conversion rules for the provided content.
pub fn text(text: impl IntoText) -> Text {
    Text::new(text)
}

impl View for Text {
    fn body(self, env: &Environment) -> impl View {
        match self.0 {
            TextKind::Raw(config) => RawText(config),
            TextKind::Signal { resolver } => {
                let config = resolver(env);
                RawText(TextConfig {
                    content: flatten_signal(config.map(|config| config.content)),
                    paragraph_alignment: flatten_signal(
                        config.map(|config| config.paragraph_alignment),
                    ),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use waterui_locale::{TranslationCatalog, locales};

    #[test]
    fn text_align_updates_paragraph_alignment_signal() {
        let text =
            Text::computed(StyledStr::plain("hello")).text_align(HorizontalAlignment::Trailing);
        assert_eq!(
            text.paragraph_alignment().get(),
            HorizontalAlignment::Trailing
        );
    }

    #[test]
    fn computed_static_str_resolves_through_i18n_catalog() {
        let env = test_env();
        let text: Text = Computed::constant("greeting").into();

        assert_eq!(resolved_plain(&text, &env), "Hello");
    }

    #[test]
    fn add_supports_computed_into_text_rhs() {
        let env = test_env();
        let text = Text::verbatim("Hi ") + Computed::constant("greeting");

        assert_eq!(resolved_plain(&text, &env), "Hi Hello");
    }

    #[test]
    fn add_assign_supports_localized_rhs() {
        let env = test_env();
        let mut text = Text::verbatim("Hi ");
        text += "greeting";

        assert_eq!(resolved_plain(&text, &env), "Hi Hello");
    }

    fn test_env() -> Environment {
        let mut env = Environment::new();
        env.insert(locales::EN);
        env.insert(
            TranslationCatalog::new()
                .add_toml("en", "greeting = \"Hello\"")
                .expect("test catalog must parse"),
        );
        env
    }

    fn resolved_plain(text: &Text, env: &Environment) -> String {
        text.resolve(env).content.get().to_plain().to_string()
    }
}
