//! Locale Example - Demonstrates WaterUI's i18n capabilities with a world fair kiosk
//!
//! This example showcases:
//! - Translation files in `i18n/` folder (TOML format)
//! - The `text!` macro for compile-time localized strings
//! - System locale detection from native side via FFI
//! - Language variant fallback (zh-TW vs zh-HK vs zh-CN)
//! - CLDR plural rules
//! - Date and unit formatting
//! - Interactive locale switching via WaterUI regional runtime callbacks
//!
//! ## How i18n Works
//!
//! 1. Define translations in `i18n/<locale>.toml` files
//! 2. Use `text!("key")` - translations are loaded at compile time
//! 3. Push locale updates to `waterui::regional`
//! 4. `text!` automatically reacts to locale changes - no `watch()` needed!

use waterui::app::App;
use waterui::form::picker::{Picker, PickerItem};
use waterui::prelude::*;
use waterui::preview;
use waterui_locale::format::date::{
    DateStyle, SimpleDate, SimpleTime, TimeStyle, format_date,
    format_datetime_with_regional_context,
};
use waterui_locale::format::unit::{Kilometer, Length, Meter};
use waterui_locale::{Locale, LocalizedDisplay, locales};

/// Available locales for the picker
fn available_locales() -> [PickerItem<&'static str>; 11] {
    [
        text("English (US)").tag("en-US"),
        text("English (UK)").tag("en-GB"),
        text("中文 (简体)").tag("zh"),
        text("中文 (台灣)").tag("zh-TW"),
        text("中文 (香港)").tag("zh-HK"),
        text("日本語").tag("ja"),
        text("한국어").tag("ko"),
        text("Deutsch").tag("de"),
        text("Français").tag("fr"),
        text("Español").tag("es"),
        text("Русский").tag("ru"),
    ]
}

/// Convert locale code to Locale
fn locale_from_code(code: &str) -> Locale {
    match code {
        "en" => locales::EN,
        "en-US" => locales::EN_US,
        "en-GB" => locales::EN_GB,
        "zh" => locales::ZH_CN,
        "zh-TW" => locales::ZH_TW,
        "zh-HK" => locales::ZH_HK,
        "ja" => locales::JA,
        "ko" => locales::KO,
        "de" => locales::DE,
        "fr" => locales::FR,
        "es" => locales::ES,
        "ru" => locales::RU,
        _ => locales::EN_US,
    }
}

/// Locale picker section
fn locale_picker_section(selection: &Binding<&'static str>, system_locale: &Locale) -> impl View {
    vstack((
        text!("Language Booth").size(16.0).bold(),
        hstack((
            text!("Detected Locale:"),
            spacer(),
            text(system_locale.to_string()),
        )),
        hstack((
            text!("Chosen Language:"),
            spacer(),
            Picker::new(text!("Chosen Language:"), available_locales(), selection).hide_label(),
        )),
    ))
}

/// Welcome desk section using text! macro
fn greeting_section() -> impl View {
    vstack((
        text!("Welcome Desk").size(16.0).bold(),
        text!("Welcome to the World Fair!").size(24.0),
    ))
}

/// Local flavor section - demonstrates variant differences (US vs UK English)
fn regional_vocab_section() -> impl View {
    vstack((
        text!("Local Flavor").size(16.0).bold(),
        hstack((spacer(), text!("Color"), spacer())),
        hstack((spacer(), text!("Favorite"), spacer())),
    ))
}

/// Human rights section - UDHR Article 1
fn paragraph_section() -> impl View {
    vstack((
        text!("Human Rights - Article 1").size(16.0).bold(),
        text!("$udhr_article_1").size(14.0),
    ))
}

/// Date formatting section
fn date_section(locale: Computed<Locale>) -> impl View {
    let date_short = locale
        .clone()
        .map(|locale| format_date(&locale, &SimpleDate::new(2006, 3, 20), DateStyle::Short))
        .computed();
    let date_long = locale
        .clone()
        .map(|locale| format_date(&locale, &SimpleDate::new(2006, 3, 20), DateStyle::Long))
        .computed();
    let timezone = locale
        .clone()
        .map(|_| waterui::regional::current_settings().timezone().to_string())
        .computed();
    let datetime_with_zone = locale
        .clone()
        .map(|_| {
            format_datetime_with_regional_context(
                &waterui::regional::current_settings(),
                &SimpleDate::new(2006, 3, 20),
                &SimpleTime::new(9, 30, 0),
                DateStyle::Long,
                TimeStyle::Long,
            )
        })
        .computed();

    vstack((
        text!("Festival Date (2006-03-20)").size(16.0).bold(),
        hstack((text!("Short:"), spacer(), text!("{date_short}"))),
        hstack((text!("Long:"), spacer(), text!("{date_long}"))),
        hstack((text!("Timezone:"), spacer(), text!("{timezone}"))),
        hstack((
            text!("Kickoff (TZ):"),
            spacer(),
            text!("{datetime_with_zone}"),
        )),
    ))
}

/// Plural section using text! macro with plural source
fn plural_section() -> impl View {
    vstack((
        text!("Passport Stamps").size(16.0).bold(),
        text!("I have {#count} passport stamp", count = 0),
        text!("I have {#count} passport stamp", count = 1),
        text!("I have {#count} passport stamp", count = 2),
        text!("I have {#count} passport stamp", count = 5),
    ))
}

/// Unit formatting section
fn unit_section(locale: Computed<Locale>) -> impl View {
    let distance = locale
        .clone()
        .map(|locale| Length::<Meter>::new(1500.0).to_localized_string(&locale))
        .computed();
    let marathon = locale
        .clone()
        .map(|locale| Length::<Kilometer>::new(42.195).to_localized_string(&locale))
        .computed();

    vstack((
        text!("Distance Guide").size(16.0).bold(),
        hstack((text!("City Walk:"), spacer(), text!("{distance}"))),
        hstack((text!("Marathon Route:"), spacer(), text!("{marathon}"))),
    ))
}

/// Content that automatically updates when runtime locale changes
/// The text! macros will react to locale changes via the environment
fn localized_content() -> impl View {
    vstack((
        greeting_section(),
        Divider,
        paragraph_section(),
        Divider,
        regional_vocab_section(),
        Divider,
        plural_section(),
    ))
}

/// Date/unit sections need the locale value for formatting APIs
fn formatted_content(locale: Computed<Locale>) -> impl View {
    let unit_locale = locale.clone();
    vstack((date_section(locale), Divider, unit_section(unit_locale)))
}

fn scene(system_locale: Locale) -> impl View {
    // Determine initial locale code from system locale
    let initial_code: &'static str = match system_locale.language.as_str() {
        "en" => match system_locale.region.as_ref().map(|r| r.as_str()) {
            Some("GB") => "en-GB",
            _ => "en-US", // Default to US English
        },
        "zh" => match system_locale.region.as_ref().map(|r| r.as_str()) {
            Some("TW") => "zh-TW",
            Some("HK") => "zh-HK",
            _ => "zh",
        },
        "ja" => "ja",
        "ko" => "ko",
        "de" => "de",
        "fr" => "fr",
        "es" => "es",
        "ru" => "ru",
        _ => "en-US",
    };

    // Create binding for selected locale code
    let selection = Binding::container(initial_code);
    // Initialize shared runtime locale from the picker's initial value.
    waterui::regional::set_locale_tag(initial_code).expect("picker locale tag must be valid");

    scroll(
        vstack((
            // Title
            text!("WaterUI World Fair").size(28.0).bold(),
            text!("Live translations for a tiny world-fair kiosk").size(14.0),
            Divider,
            // Locale picker
            locale_picker_section(&selection, &system_locale),
            Divider,
            // Localized content - text! macros react to regional locale updates
            // No watch() needed for these!
            localized_content(),
            Divider,
            // Formatting APIs receive a computed locale, so only their text
            // leaves update when the selection changes.
            formatted_content(selection.clone().map(locale_from_code).computed()),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
    .on_change(&selection, |code| {
        waterui::regional::set_locale_tag(code).expect("picker locale tag must be valid");
    })
}

/// Self-contained entry: starts from US English and lets the picker drive the
/// runtime locale. Used for embedding (gallery) and `water preview`.
#[preview]
pub fn demo() -> impl View {
    scene(locales::EN_US.clone())
}

pub fn app(env: Environment) -> App {
    let system_locale = env
        .get::<Locale>()
        .cloned()
        .unwrap_or_else(|| locales::EN.clone());

    App::new(move || scene(system_locale.clone()), env)
}
