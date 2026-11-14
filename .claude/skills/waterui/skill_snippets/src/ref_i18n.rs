//! Snippets from `.claude/skills/waterui/references/i18n.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! The crate carries a real `i18n/en.toml` and `i18n/de.toml` whose keys are
//! exactly the literals these snippets use — including the CLDR plural tables —
//! so the macro parses them at compile time rather than falling through to the
//! literal.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// i18n.md § "## How translation works" — rust block 1/7
// Listing: two independent lookups.
// ---------------------------------------------------------------------------
pub fn i18n_block_01() {
    let name = Binding::container(Str::from("Ada"));

    let _ = {
        text!("Welcome Desk") // looked up verbatim as the key "Welcome Desk"
    };
    let _ = {
        text!("Hello, {name}") // placeholders are slot keys inside the translation
    };
}

// ---------------------------------------------------------------------------
// i18n.md § "## How translation works" (prose): `text("..")` on a
// `&'static str` is a runtime catalog lookup, `String` / `Str` /
// `Text::verbatim` are verbatim, `waterui::configure_environment!` installs the
// catalog on a hand-built environment, `{blur:.1}` is a format spec, and
// `@ context` appends `#context` to the key, and `Text::localized_or` /
// `Text::localized_with` take a fallback / a locale-aware resolver.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
// The prose names `waterui::configure_environment!` with its full path.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn i18n_translation_prose() {
    use waterui::text::{StyledStr, Text};

    let radius = Binding::container(1.5_f64);

    let _ = text("Welcome Desk");
    let _ = text(String::from("Welcome Desk"));
    let _ = text(Str::from("Welcome Desk"));
    let _ = Text::verbatim("Welcome Desk");
    let _ = waterui::configure_environment!(Environment::new());
    let _ = text!("Blur: {blur:.1}", blur = radius);
    let _ = text!("Open" @ menu);
    let _ = text!("Open" @ "file");
    let _ = Text::localized_or("nav.back", "Back");
    let _ = Text::localized_with(|_env, _locale| TextConfig::new(StyledStr::plain("Back")));
}

// ---------------------------------------------------------------------------
// i18n.md § "## The catalog files" (prose): `$`-keys are referenced as
// `text!("$about_blurb")`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_dollar_key_prose() {
    let _ = text!("$about_blurb");
}

// ---------------------------------------------------------------------------
// i18n.md § "## Plurals" — rust block 2/7
// ---------------------------------------------------------------------------
pub fn i18n_block_02() -> impl View {
    let stamp_count = Binding::i32(2);

    text!("I have {#count} passport stamp", count = stamp_count)
}

// ---------------------------------------------------------------------------
// i18n.md § "## Plurals" — rust block 3/7
// ---------------------------------------------------------------------------
pub fn i18n_block_03() -> impl View {
    let file_count = Binding::i32(1);
    let folder_count = Binding::i32(3);

    text!(
        "{#files} in {#folders}",
        files = file_count,
        folders = folder_count
    )
}

// ---------------------------------------------------------------------------
// i18n.md § "## Switching locale at runtime" — rust block 4/7
// ---------------------------------------------------------------------------
// i18n.md writes the path qualified to show where the function lives.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn i18n_block_04() {
    use waterui::locale::{Locale, locales};

    waterui::regional::set_locale_tag("zh-TW").expect("valid BCP-47 tag");

    let _: Option<Locale> = None;
    let _ = locales::EN;
}

// ---------------------------------------------------------------------------
// i18n.md § "## Switching locale at runtime" (prose): `locales::` constants
// used directly (`Binding::container(locales::JA)`), `Locale`'s `language` /
// `region` fields with `.as_str()` (through its `Deref`), `canonical_tag()`,
// its `FromStr` impl, and `waterui::locale::locale_binding(&env)`. Not counted
// as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_locale_api_prose(env: &Environment) {
    use waterui::locale::{Locale, locale_binding, locales};

    let _ = locales::EN;
    let _ = locales::EN_US;
    let _ = locales::ZH_TW;
    let _ = locales::AR;
    let _ = Binding::container(locales::JA);

    let l = locales::EN_US;
    let _ = l.language.as_str();
    let _: Option<_> = l.region.as_ref().map(|r| r.as_str());
    let _: String = l.canonical_tag();

    let _ = "he".parse::<Locale>().expect("valid tag");

    let _: Binding<Locale> = locale_binding(env);
}

// ---------------------------------------------------------------------------
// i18n.md § "## The system locale" — rust block 5/7
// ---------------------------------------------------------------------------
pub mod i18n_block_05 {
    use waterui::app::App;
    use waterui::locale::{Locale, locale_binding};
    use waterui::prelude::*;

    fn scene(_locale: Binding<Locale>) -> impl View {
        text("scene")
    }

    pub fn app(env: Environment) -> App {
        let locale = locale_binding(&env);
        App::new(move || scene(locale.clone()), env)
    }
}

// ---------------------------------------------------------------------------
// i18n.md § "## The system locale" (prose): `regional::current_settings()
// .locale()` is the same value as a plain snapshot. Not counted as a rust
// block.
// ---------------------------------------------------------------------------
// The prose names `waterui::regional::current_settings()` with its full path.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn i18n_current_settings_locale_prose() {
    let _ = waterui::regional::current_settings().locale().clone();
}

// ---------------------------------------------------------------------------
// i18n.md § "## Scoping locale and direction to a subtree" — rust block 6/7
// Listing: three independent scoping forms.
// ---------------------------------------------------------------------------
pub fn i18n_block_06() {
    use waterui::locale::locales;

    fn arabic_panel() -> impl View {
        text("لوحة")
    }
    fn panel() -> impl View {
        text("panel")
    }

    use waterui::env::with;

    let _ = {
        with(text!("Article").body(), Binding::container(locales::JA)) // Japanese glyph selection
    };
    let _ = {
        with(arabic_panel(), LayoutDirection::RightToLeft) // RTL for this subtree only
    };
    let _ = {
        // both, nested
        with(
            with(panel(), Binding::container(locales::AR)),
            LayoutDirection::RightToLeft,
        )
    };
}

// ---------------------------------------------------------------------------
// i18n.md § "## Scoping locale and direction to a subtree" (prose):
// `view.with(value)` is the method spelling. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_with_method_prose() {
    let _ = text("panel").with(LayoutDirection::RightToLeft);
}

// ---------------------------------------------------------------------------
// i18n.md § "## Locale-aware formatting" — rust block 7/7
// ---------------------------------------------------------------------------
pub fn i18n_block_07() -> impl View {
    use waterui::locale::locales;

    let locale = Binding::container(locales::EN_US);

    use waterui::locale::LocalizedDisplay; // trait import required
    use waterui::locale::format::date::{DateStyle, SimpleDate, format_date};
    use waterui::locale::format::unit::{Length, Meter};

    let date = locale
        .map(|l| format_date(&l, &SimpleDate::new(2026, 3, 20), DateStyle::Long))
        .computed();
    let distance = locale
        .map(|l| Length::<Meter>::new(1500.0).to_localized_string(&l))
        .computed();
    vstack((text!("{date}"), text!("{distance}")))
}

// ---------------------------------------------------------------------------
// i18n.md § "## Locale-aware formatting" (prose): `SimpleTime::new(h, m, s)`,
// `TimeStyle`, `format_time`, `format_datetime`,
// `format_datetime_with_regional_context(&ctx, ..)`, explicit unit conversion
// with `.to::<Mile>()`, the `Length` / `Temperature` / `Mass` unit types
// in `format::unit`, the crate-root siblings `format_number` /
// `format_percent` / `format_currency` / `LocalizedList`, and
// `waterui::regional::current_settings()`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
// The prose names `waterui::regional::current_settings()` with its full path.
#[allow(unknown_lints, qualified_waterui_path)]
pub fn i18n_formatting_siblings_prose() {
    use waterui::locale::format::date::{
        DateStyle, SimpleDate, SimpleTime, TimeStyle, format_datetime,
        format_datetime_with_regional_context, format_time,
    };
    use waterui::locale::format::number::Currency;
    use waterui::locale::format::unit::{
        Celsius, Fahrenheit, Feet, Gram, Kelvin, Kilogram, Kilometer, Length, Mass, Meter, Mile,
        Ounce, Pound, Temperature,
    };
    use waterui::locale::{
        LocalizedDisplay, LocalizedList, format_currency, format_number, format_percent, locales,
    };

    let l = locales::EN_US;
    let time = SimpleTime::new(13, 5, 0);
    let date = SimpleDate::new(2026, 3, 20);
    let _ = format_time(&l, &time, TimeStyle::Short);
    let _ = format_datetime(&l, &date, &time, DateStyle::Long, TimeStyle::Short);
    let _ = format_datetime_with_regional_context(
        &waterui::regional::current_settings(),
        &date,
        &time,
        DateStyle::Long,
        TimeStyle::Short,
    );
    let _ = Length::<Kilometer>::new(42.195).to::<Mile>();
    let _ = Length::<Meter>::new(3.0).to::<Feet>();
    let _ = Temperature::<Celsius>::new(21.0).to_localized_string(&l);
    let _ = Mass::<Kilogram>::new(2.0).to_localized_string(&l);
    let _ = Temperature::<Fahrenheit>::new(70.0).to::<Kelvin>();
    let _ = Mass::<Gram>::new(500.0).to::<Pound>().to::<Ounce>();
    let _ = format_number(&l, 1234.5);
    let _ = format_percent(&l, 0.42);
    let _ = format_currency(&l, 19.99, Currency::USD);
    let _ = LocalizedList(&["a", "b", "c"]).to_localized_string(&l);
    let _ = waterui::regional::current_settings();
}

// ---------------------------------------------------------------------------
// i18n.md § "## Right-to-left layout" (prose): `LayoutDirection` is in the
// prelude with `LeftToRight` / `RightToLeft`; `waterui::locale::layout_direction`
// derives one from a `Locale`; a `LayoutDirection` / `Binding<LayoutDirection>`
// in the environment overrides the locale-derived direction.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_layout_direction_prose() {
    let _ = LayoutDirection::LeftToRight;
    let _ = LayoutDirection::RightToLeft;

    let _: LayoutDirection = waterui::locale::layout_direction(&waterui::locale::locales::AR);

    let mut env = Environment::new();
    env.insert(LayoutDirection::RightToLeft);
    env.insert(Binding::container(LayoutDirection::RightToLeft));
}
