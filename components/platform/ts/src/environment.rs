//! The framework values a mounted module sees.
//!
//! `Environment` is keyed by Rust type, so an application's own values are
//! structurally unreachable from TypeScript — by design: application data
//! travels through typed props. What the framework owns does cross, as the
//! built-in contexts `useTheme()` and `useLocale()` resolve, and this module
//! is where those values are read out of the `Environment` the runtime was
//! handed and projected into the shapes HOST.md documents.
//!
//! There is no safe-area context. `WaterUI` publishes no ambient inset value: a
//! backend places content clear of the hardware at the container level, so
//! neither the framework nor a view ever reads an inset number, and an
//! accessor that could only answer zeroes would fake a primitive that does not
//! exist.

use nami::watcher::BoxWatcherGuard;
use nami::{Binding, Computed, Signal, SignalExt};
use waterui_core::Environment;
use waterui_core::layout::LayoutDirection;
use waterui_graphics::ResolvedColor;
use waterui_graphics::color::ColorScheme;
use waterui_locale::{Locale, layout_direction, locale_binding};
use waterui_ts_engine::{JsError, JsValue};

use crate::bridge::Bridge;
use crate::convert::IntoJs;
use crate::tether::Tethered;

/// The appearance a mounted module renders in.
///
/// One value, not one signal per token: the theme reaches TypeScript as a
/// single reactive object, so a palette swap arrives as one change rather than
/// fifteen.
#[derive(Debug, Clone)]
pub struct Theme {
    /// The light or dark appearance the application is drawn in.
    pub color_scheme: ColorScheme,
    /// The colour tokens the environment installs, in the fixed token order.
    ///
    /// A token the environment does not install is absent rather than
    /// defaulted: a theme that never declared a colour has none, and inventing
    /// one would put a value on screen that no theme chose.
    pub colors: Vec<(&'static str, ResolvedColor)>,
}

/// The locale a mounted module formats in, in the shape `useLocale()` reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostLocale {
    /// The canonical BCP-47 tag, for example `"en-US"`.
    pub identifier: String,
    /// The language subtag alone, for example `"en"`.
    pub language_code: String,
    /// `"ltr"` or `"rtl"`, resolved from the locale's script.
    pub text_direction: &'static str,
}

impl From<&Locale> for HostLocale {
    fn from(locale: &Locale) -> Self {
        Self {
            identifier: locale.canonical_tag(),
            language_code: locale.language.to_string(),
            text_direction: match layout_direction(locale) {
                LayoutDirection::RightToLeft => "rtl",
                LayoutDirection::LeftToRight => "ltr",
            },
        }
    }
}

impl IntoJs for ColorScheme {
    /// `"light"` or `"dark"`, the union `useTheme()` is typed against.
    fn into_js(self, _bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::String(
            match self {
                Self::Light => "light",
                Self::Dark => "dark",
            }
            .to_owned(),
        ))
    }
}

impl IntoJs for ResolvedColor {
    /// The five channels of a resolved colour, in linear light: `red`,
    /// `green` and `blue` leave 0–1 for a wide-gamut colour, and `headroom`
    /// above 1 marks an extended-range one.
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Object(vec![
            (String::from("red"), self.red.into_js(bridge)?),
            (String::from("green"), self.green.into_js(bridge)?),
            (String::from("blue"), self.blue.into_js(bridge)?),
            (String::from("headroom"), self.headroom.into_js(bridge)?),
            (String::from("opacity"), self.opacity.into_js(bridge)?),
        ]))
    }
}

impl IntoJs for Theme {
    /// `{ colorScheme, …tokens }` — the tokens sit beside the scheme, which is
    /// the shape `contexts.d.ts` declares.
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        let mut entries = vec![(
            String::from("colorScheme"),
            self.color_scheme.into_js(bridge)?,
        )];
        for (name, color) in self.colors {
            entries.push((String::from(name), color.into_js(bridge)?));
        }
        Ok(JsValue::Object(entries))
    }
}

impl IntoJs for HostLocale {
    fn into_js(self, bridge: &Bridge) -> Result<JsValue, JsError> {
        Ok(JsValue::Object(vec![
            (String::from("identifier"), self.identifier.into_js(bridge)?),
            (
                String::from("languageCode"),
                self.language_code.into_js(bridge)?,
            ),
            (
                String::from("textDirection"),
                self.text_direction.into_js(bridge)?,
            ),
        ]))
    }
}

/// The colour tokens a theme installs, and the property each projects to.
macro_rules! theme_tokens {
    ($($key:ident => $name:literal),* $(,)?) => {
        /// Every installed token, in declaration order.
        fn token_sources(environment: &Environment) -> Vec<(&'static str, Computed<ResolvedColor>)> {
            let mut sources = Vec::new();
            $(
                if let Some(color) = environment
                    .query::<waterui_graphics::color::$key, Computed<ResolvedColor>>()
                {
                    sources.push(($name, color.clone()));
                }
            )*
            sources
        }
    };
}

theme_tokens!(
    ForegroundColor => "foreground",
    BackgroundColor => "background",
    SurfaceColor => "surface",
    SurfaceVariantColor => "surfaceVariant",
    BorderColor => "border",
    AccentColor => "accent",
    MutedForegroundColor => "mutedForeground",
    AccentForegroundColor => "accentForeground",
    AccentContainerColor => "accentContainer",
    TertiaryColor => "tertiary",
    TertiaryContainerColor => "tertiaryContainer",
    SelectionContainerColor => "selectionContainer",
    SelectionForegroundColor => "selectionForeground",
    ErrorColor => "error",
    ErrorForegroundColor => "errorForeground",
);

/// The reactive theme of `environment`.
///
/// The scheme and every installed token are watched, and each change rebuilds
/// the snapshot, so the TypeScript side sees one value that is always whole.
///
/// # Errors
///
/// Returns [`JsError`] when no `ColorScheme` is installed. A backend installs
/// one before it mounts anything — light or dark is the one fact every theme
/// has — and reporting a default here would draw a light theme over a dark
/// application.
pub fn theme(environment: &Environment) -> Result<Computed<Theme>, JsError> {
    let scheme = environment
        .query::<ColorScheme, Computed<ColorScheme>>()
        .cloned()
        .ok_or_else(|| {
            JsError::new(
                "Error",
                "this environment installs no ColorScheme, so a TypeScript module has no theme \
                 to read: a backend installs the appearance before it mounts a view",
            )
        })?;
    let tokens = token_sources(environment);

    let snapshot = {
        let scheme = scheme.clone();
        let tokens = tokens.clone();
        move || Theme {
            color_scheme: scheme.get(),
            colors: tokens
                .iter()
                .map(|(name, color)| (*name, color.get()))
                .collect(),
        }
    };

    let binding = Binding::container(snapshot());
    let mut guards: Vec<BoxWatcherGuard> = Vec::with_capacity(tokens.len() + 1);
    guards.push(scheme.watch({
        let binding = binding.clone();
        let snapshot = snapshot.clone();
        move |_| binding.set(snapshot())
    }));
    for (_, color) in &tokens {
        guards.push(color.watch({
            let binding = binding.clone();
            let snapshot = snapshot.clone();
            move |_| binding.set(snapshot())
        }));
    }

    Ok(Computed::new(Tethered::new(
        binding,
        std::rc::Rc::new(guards),
    )))
}

/// The reactive locale of `environment`, projected for TypeScript.
#[must_use]
pub fn locale(environment: &Environment) -> Computed<HostLocale> {
    locale_binding(environment)
        .map(|locale| HostLocale::from(&locale))
        .computed()
}

/// `environment()`: the framework values one mount publishes as contexts.
///
/// # Errors
///
/// Returns [`JsError`] when the theme cannot be read, or when either value
/// cannot be exported.
pub fn host_environment(bridge: &Bridge) -> Result<JsValue, JsError> {
    let theme = theme(bridge.environment())?.into_js(bridge)?;
    let locale = locale(bridge.environment()).into_js(bridge)?;
    Ok(JsValue::Object(vec![
        (String::from("theme"), theme),
        (String::from("locale"), locale),
    ]))
}
