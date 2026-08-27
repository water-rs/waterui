//! Snippets from `.claude/skills/waterui/references/i18n.md`, in file order.
//! Transcription conventions are documented in the crate README.
//!
//! The crate carries a real `i18n/en.toml` and `i18n/de.toml` whose keys are
//! exactly the literals these snippets use — including the CLDR plural table —
//! so the macro parses them at compile time rather than falling through to the
//! literal.

use waterui::prelude::*;

// ---------------------------------------------------------------------------
// i18n.md § "## How translation works" — rust block 1/6
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
// i18n.md § "## The catalog files" (prose): `$`-keys are referenced as
// `text!("$about_blurb")`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_dollar_key_prose() {
    let _ = text!("$about_blurb");
}

// ---------------------------------------------------------------------------
// i18n.md § "## Plurals" — rust block 2/6
// ---------------------------------------------------------------------------
pub fn i18n_block_02() -> impl View {
    let stamp_count = Binding::i32(2);

    text!("I have {#count} passport stamp", count = stamp_count)
}

// ---------------------------------------------------------------------------
// i18n.md § "## Switching locale at runtime" — rust block 3/6
// ---------------------------------------------------------------------------
pub fn i18n_block_03() {
    use waterui::locale::{Locale, locales};

    waterui::regional::set_locale_tag("zh-TW").expect("valid BCP-47 tag");

    let _: Option<Locale> = None;
    let _ = locales::EN.clone();
}

// ---------------------------------------------------------------------------
// i18n.md § "## Switching locale at runtime" (prose): `locales::` constants,
// `Locale`'s `language` / `region` fields with `.as_str()`, and its `FromStr`
// impl. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_locale_api_prose() {
    use waterui::locale::{Locale, locales};

    let _ = locales::EN_US.clone();
    let _ = locales::ZH_TW.clone();
    let _ = locales::JA.clone();
    let _ = locales::AR.clone();

    let l = locales::EN_US.clone();
    let _ = l.language.as_str();
    let _: Option<_> = l.region.as_ref().map(|r| r.as_str());

    let _ = "he".parse::<Locale>().expect("valid tag");
}

// ---------------------------------------------------------------------------
// i18n.md § "## The system locale" — rust block 4/6
// ---------------------------------------------------------------------------
pub mod i18n_block_04 {
    use waterui::app::App;
    use waterui::locale::{Locale, locales};
    use waterui::prelude::*;

    fn scene(_system: Locale) -> impl View {
        text("scene")
    }

    pub fn app(env: Environment) -> App {
        let system = env
            .get::<Locale>()
            .cloned()
            .unwrap_or_else(|| locales::EN.clone());
        App::new(move || scene(system.clone()), env)
    }
}

// ---------------------------------------------------------------------------
// i18n.md § "## Scoping locale and direction to a subtree" — rust block 5/6
// Listing: three independent scoping forms.
// ---------------------------------------------------------------------------
pub fn i18n_block_05() {
    use waterui::locale::locales;

    fn arabic_panel() -> impl View {
        text("لوحة")
    }
    fn panel() -> impl View {
        text("panel")
    }

    use waterui::env::with;

    let _ = {
        with(text!("Article").body(), locales::JA.clone()) // Japanese glyph selection
    };
    let _ = {
        with(arabic_panel(), LayoutDirection::RightToLeft) // RTL for this subtree only
    };
    let _ = {
        // both, nested
        with(
            with(panel(), locales::AR.clone()),
            LayoutDirection::RightToLeft,
        )
    };
}

// ---------------------------------------------------------------------------
// i18n.md § "## Locale-aware formatting" — rust block 6/6
// ---------------------------------------------------------------------------
pub fn i18n_block_06() -> impl View {
    use waterui::locale::locales;

    let locale = Binding::container(locales::EN_US.clone());

    use waterui_locale::LocalizedDisplay; // trait import required
    use waterui_locale::format::date::{DateStyle, SimpleDate, format_date};
    use waterui_locale::format::unit::{Length, Meter};

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
// the crate-root siblings `format_number` / `format_percent` /
// `format_currency`, and `waterui::regional::current_settings()`.
// Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_formatting_siblings_prose() {
    use waterui::locale::locales;
    use waterui_locale::format::date::SimpleTime;
    use waterui_locale::format::number::Currency;
    use waterui_locale::{format_currency, format_number, format_percent};

    let l = locales::EN_US.clone();
    let _ = SimpleTime::new(13, 5, 0);
    let _ = format_number(&l, 1234.5);
    let _ = format_percent(&l, 0.42);
    let _ = format_currency(&l, 19.99, Currency::USD);
    let _ = waterui::regional::current_settings();
}

// ---------------------------------------------------------------------------
// i18n.md § "## Right-to-left layout" (prose): `LayoutDirection` is in the
// prelude with `LeftToRight` / `RightToLeft`. Not counted as a rust block.
// ---------------------------------------------------------------------------
pub fn i18n_layout_direction_prose() {
    let _ = LayoutDirection::LeftToRight;
    let _ = LayoutDirection::RightToLeft;
}
