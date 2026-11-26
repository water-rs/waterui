//! The two environment locale resolvers must agree on precedence when an
//! environment carries both a `RegionalContext` (installed by the Apple
//! backend after `set_locale_tag`) and an explicit `Locale` override.

use nami::{Binding, Signal as _};
use waterui_core::{Environment, extract::Extractor};
use waterui_locale::{Locale, locale_binding, locales, regional};

#[test]
fn repro_7_resolvers_agree_on_locale_vs_regional_context() {
    regional::set_locale_tag("en-US").unwrap();
    let mut env = Environment::new();
    env.insert(regional::current_settings()); // en-US, as the backend installs it
    env.insert(locales::JA); // `with(view, locales::JA)`

    let extracted = Locale::extract(&env).unwrap().canonical_tag();
    let bound = locale_binding(&env);
    let bound = bound.snapshot().canonical_tag();
    assert_eq!(
        extracted, bound,
        "Extractor says {extracted}, locale_binding says {bound}"
    );
    assert_eq!(extracted, "ja");
}

#[test]
fn locale_extract_and_binding_share_full_precedence_across_cjk_and_regional_locales() {
    regional::set_locale_tag("en-US").unwrap();
    let ambient = regional::current_settings();

    for override_locale in [
        locales::ZH_CN,
        locales::JA,
        locales::KO,
        locales::DE,
        "ar-EG".parse().unwrap(),
    ] {
        let mut env = Environment::new();
        env.insert(ambient.clone());
        assert_eq!(Locale::extract(&env).unwrap().canonical_tag(), "en-US");
        assert_eq!(locale_binding(&env).snapshot().canonical_tag(), "en-US");

        env.insert(override_locale.clone());
        let expected = override_locale.canonical_tag();
        assert_eq!(Locale::extract(&env).unwrap().canonical_tag(), expected);
        assert_eq!(locale_binding(&env).snapshot().canonical_tag(), expected);

        let binding = Binding::new(locales::EN_US);
        env.insert(binding.clone());
        assert_eq!(Locale::extract(&env).unwrap().canonical_tag(), "en-US");
        assert_eq!(locale_binding(&env).snapshot().canonical_tag(), "en-US");

        binding.set(override_locale);
        assert_eq!(Locale::extract(&env).unwrap().canonical_tag(), expected);
        assert_eq!(locale_binding(&env).snapshot().canonical_tag(), expected);
    }
}
