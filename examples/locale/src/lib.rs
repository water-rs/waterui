//! Locale Example - Demonstrates `WaterUI`'s i18n capabilities with a world fair kiosk
//!
//! This example showcases:
//! - Translation files in `i18n/` folder (TOML format)
//! - The `text!` macro for compile-time localized strings
//! - System locale detection from native side via FFI
//! - Language variant fallback (zh-TW vs zh-HK vs zh-CN)
//! - CLDR plural rules
//! - Date and unit formatting
//! - Interactive locale switching via `WaterUI` regional runtime callbacks
//!
//! ## How i18n Works
//!
//! 1. Define translations in `i18n/<locale>.toml` files
//! 2. Use `text!("key")` - translations are loaded at compile time
//! 3. Push locale updates to `waterui::regional`
//! 4. `text!` automatically reacts to locale changes - no `watch()` needed!

use jiff::civil;
use waterui::app::{App, Windows};
use waterui::catalog;
use waterui::form::picker::{Picker, PickerItem};
use waterui::prelude::*;
use waterui::preview;
use waterui::regional::current_settings;
use waterui::regional::set_locale_tag;
use waterui_locale::format::date::{DateStyle, TimeStyle, format_datetime_with_regional_context};
use waterui_locale::format::unit::{Kilometer, Length, Meter};
use waterui_locale::locale::get_fallback_chain;
use waterui_locale::{Locale, LocalizedDisplay, RelativeTime, TranslationCatalog, locales};

/// Builds the locale picker items dynamically from the compiled translation catalog.
fn available_locales(catalog: &TranslationCatalog) -> Vec<PickerItem<Locale>> {
    catalog
        .locales()
        .map(|locale| text(locale.display_name(locale)).tag(locale.clone()))
        .collect()
}

/// Resolves the best matching catalog locale for the host's system locale.
fn initial_locale(catalog: &TranslationCatalog, system_locale: &Locale) -> Locale {
    get_fallback_chain(system_locale)
        .into_iter()
        .find_map(|candidate| {
            catalog
                .locales()
                .find(|locale| locale.id() == candidate.id())
                .cloned()
        })
        .or_else(|| {
            catalog
                .locales()
                .find(|locale| locale.id() == locales::EN_US.id())
                .cloned()
        })
        .or_else(|| catalog.locales().next().cloned())
        .unwrap_or(locales::EN_US)
}

/// Locale picker section
fn locale_picker_section(
    catalog: &TranslationCatalog,
    selection: &Binding<Locale>,
    detected: String,
) -> impl View + use<> {
    vstack((
        text!("Language Booth").size(16.0).bold(),
        hstack((text!("Detected Locale:"), spacer(), text(detected))),
        hstack((
            text!("Chosen Language:"),
            spacer(),
            Picker::new(
                text!("Chosen Language:"),
                available_locales(catalog),
                selection,
            )
            .hide_label(),
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
fn date_section(locale: &Computed<Locale>) -> impl View + use<> {
    let festival = civil::date(2006, 3, 20);
    let date_short = festival.formatted(DateStyle::Short);
    let date_long = festival.formatted(DateStyle::Long);
    let opens_in = RelativeTime::days(3);
    let timezone = locale
        .clone()
        .map(|_| current_settings().timezone().to_string())
        .computed();
    let datetime_with_zone = locale
        .clone()
        .map(move |_| {
            format_datetime_with_regional_context(
                &current_settings(),
                festival.at(9, 30, 0, 0),
                DateStyle::Long,
                TimeStyle::Long,
            )
        })
        .computed();

    vstack((
        text!("Festival Date (2006-03-20)").size(16.0).bold(),
        hstack((text!("Short:"), spacer(), text!("{date_short}"))),
        hstack((text!("Long:"), spacer(), text!("{date_long}"))),
        hstack((text!("Countdown:"), spacer(), text!("{opens_in}"))),
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
fn unit_section(locale: &Computed<Locale>) -> impl View + use<> {
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
fn formatted_content(locale: &Computed<Locale>) -> impl View + use<> {
    vstack((date_section(locale), Divider, unit_section(locale)))
}

fn scene(system_locale: &Locale) -> impl View + use<> {
    let catalog = catalog!();
    let initial = initial_locale(&catalog, system_locale);

    // Create binding for the selected locale.
    let selection = Binding::container(initial.clone());
    // Initialize shared runtime locale from the picker's initial value.
    set_locale_tag(initial.canonical_tag()).expect("picker locale tag must be valid");
    let formatted_locale = selection.computed();

    scroll(
        vstack((
            // Title
            text!("WaterUI World Fair").size(28.0).bold(),
            text!("Live translations for a tiny world-fair kiosk").size(14.0),
            Divider,
            // Locale picker
            locale_picker_section(&catalog, &selection, system_locale.to_string()),
            Divider,
            // Localized content - text! macros react to regional locale updates
            // No watch() needed for these!
            localized_content(),
            Divider,
            // Formatting APIs receive a computed locale, so only their text
            // leaves update when the selection changes.
            formatted_content(&formatted_locale),
        ))
        .padding_with(16.0),
    )
    .on_change(&selection, |locale: Locale| {
        set_locale_tag(locale.canonical_tag()).expect("picker locale tag must be valid");
    })
}

/// Self-contained entry: starts from US English and lets the picker drive the
/// runtime locale. Used for embedding (gallery) and `water preview`.
#[preview]
pub fn demo() -> impl View {
    scene(&locales::EN_US)
}

/// Builds the locale example app, starting from the environment's locale.
#[must_use]
pub fn app(env: Environment) -> App<impl Windows> {
    let system_locale = env.get::<Locale>().cloned().unwrap_or(locales::EN);

    App::new(move || scene(&system_locale), env)
}
