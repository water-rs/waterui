//! Locale Example - Demonstrates WaterUI's i18n capabilities
//!
//! This example showcases:
//! - Translation files in `i18n/` folder (TOML format)
//! - The `text!` macro for compile-time localized strings
//! - System locale detection from native side via FFI
//! - Language variant fallback (zh-TW vs zh-HK vs zh-CN)
//! - CLDR plural rules
//! - Date and unit formatting
//! - Interactive locale switching with reactive `Computed<Locale>`
//!
//! ## How i18n Works
//!
//! 1. Define translations in `i18n/<locale>.toml` files
//! 2. Use `text!("key")` - translations are loaded at compile time
//! 3. Put `Computed<Locale>` in environment for reactive updates
//! 4. `text!` automatically reacts to locale changes - no `watch()` needed!

use waterui::app::App;
use waterui::form::picker::{Picker, PickerItem};
use waterui::prelude::*;
use waterui::reactive::{Computed, SignalExt};
use waterui_locale::format::date::{DateStyle, SimpleDate, format_date};
use waterui_locale::format::unit::{Kilometer, Length, Meter};
use waterui_locale::text;
use waterui_locale::{Locale, LocalizedDisplay, locales};

/// Available locales for the picker
fn available_locales() -> Vec<PickerItem<&'static str>> {
    vec![
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
        "en" => locales::EN.clone(),
        "en-US" => locales::EN_US.clone(),
        "en-GB" => locales::EN_GB.clone(),
        "zh" => locales::ZH_CN.clone(),
        "zh-TW" => locales::ZH_TW.clone(),
        "zh-HK" => locales::ZH_HK.clone(),
        "ja" => locales::JA.clone(),
        "ko" => locales::KO.clone(),
        "de" => locales::DE.clone(),
        "fr" => locales::FR.clone(),
        "es" => locales::ES.clone(),
        "ru" => locales::RU.clone(),
        _ => locales::EN_US.clone(),
    }
}

/// Locale picker section
fn locale_picker_section(selection: &Binding<&'static str>, system_locale: &Locale) -> impl View {
    vstack((
        text!("Language Selection").size(16.0).bold(),
        hstack((
            text!("System Locale:"),
            spacer(),
            text(system_locale.to_string()),
        )),
        hstack((
            text!("Selected:"),
            spacer(),
            Picker::new(available_locales(), selection),
        )),
    ))
}

/// Greeting section using text! macro
fn greeting_section() -> impl View {
    vstack((
        text!("Greeting").size(16.0).bold(),
        text!("Hello, World!").size(24.0),
    ))
}

/// Regional vocabulary section - demonstrates variant differences (US vs UK English)
fn regional_vocab_section() -> impl View {
    vstack((
        text!("Regional Vocabulary").size(16.0).bold(),
        hstack((text!("Color:"), spacer(), text!("color"))),
        hstack((text!("Favorite:"), spacer(), text!("favorite"))),
    ))
}

/// Famous paragraph section - US Constitution Preamble
fn paragraph_section() -> impl View {
    vstack((
        text!("Famous Paragraph").size(16.0).bold(),
        text!("$constitution_preamble").size(14.0),
    ))
}

/// Date formatting section
fn date_section(locale: Locale) -> impl View {
    let date = SimpleDate::new(2006, 3, 20);
    let date_short = format_date(&locale, &date, DateStyle::Short);
    let date_long = format_date(&locale, &date, DateStyle::Long);

    vstack((
        text!("Date Formatting (2006-03-20)").size(16.0).bold(),
        hstack((text!("Short:"), spacer(), text(date_short))),
        hstack((text!("Long:"), spacer(), text(date_long))),
    ))
}

/// Plural rules section using text! macro with plural source
fn plural_section() -> impl View {
    vstack((
        text!("Plural Rules").size(16.0).bold(),
        text!("I have {#count} apple", count = 0),
        text!("I have {#count} apple", count = 1),
        text!("I have {#count} apple", count = 2),
        text!("I have {#count} apple", count = 5),
    ))
}

/// Unit formatting section
fn unit_section(locale: Locale) -> impl View {
    let distance = Length::<Meter>::new(1500.0);
    let marathon = Length::<Kilometer>::new(42.195);

    vstack((
        text!("Unit Formatting").size(16.0).bold(),
        hstack((
            text!("1500m:"),
            spacer(),
            text(distance.to_localized_string(&locale)),
        )),
        hstack((
            text!("Marathon:"),
            spacer(),
            text(marathon.to_localized_string(&locale)),
        )),
    ))
}

/// Content that automatically updates when Computed<Locale> changes
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
fn formatted_content(locale: Locale) -> impl View {
    let locale_for_unit = locale.clone();
    vstack((date_section(locale), Divider, unit_section(locale_for_unit)))
}

#[hot_reload]
fn main(env: &Environment) -> impl View {
    // Get system locale from environment (injected by native FFI)
    let system_locale = env
        .get::<Locale>()
        .cloned()
        .unwrap_or_else(|| locales::EN.clone());

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
    let sel_watch = selection.clone();
    let sel_for_computed = selection.clone();

    // Create Computed<Locale> from the selection - this is what makes text! reactive!
    let locale_computed: Computed<Locale> = sel_for_computed
        .map(|code| locale_from_code(code))
        .computed();

    scroll(
        vstack((
            // Title
            text!("WaterUI i18n Example").size(28.0).bold(),
            text!("text! macros react to Computed<Locale> automatically").size(14.0),
            Divider,
            // Locale picker
            locale_picker_section(&selection, &system_locale),
            Divider,
            // Localized content - text! macros react to Computed<Locale> in environment
            // No watch() needed for these!
            localized_content(),
            Divider,
            // Date/unit formatting requires the locale value (not just text!)
            // So we still need watch() for these
            watch(sel_watch, move |code| {
                formatted_content(locale_from_code(code))
            }),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
    // Inject Computed<Locale> into environment for reactive text! macros
    .with(locale_computed)
}

pub fn app(env: Environment) -> App {
    App::new(main(&env), env)
}

waterui_ffi::export!();
