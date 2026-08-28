use waterui_core::Environment;
use waterui_locale::{TranslationCatalog, locale_binding};

const CATALOGS: &[(&str, &str)] = &[
    ("en", include_str!("../i18n/en.toml")),
    ("zh", include_str!("../i18n/zh-Hans.toml")),
    ("zh-Hans", include_str!("../i18n/zh-Hans.toml")),
    ("zh-Hant", include_str!("../i18n/zh-Hant.toml")),
    ("ja", include_str!("../i18n/ja.toml")),
    ("ko", include_str!("../i18n/ko.toml")),
    ("ar", include_str!("../i18n/ar.toml")),
    ("he", include_str!("../i18n/he.toml")),
    ("de", include_str!("../i18n/de.toml")),
    ("fr", include_str!("../i18n/fr.toml")),
    ("es", include_str!("../i18n/es.toml")),
    ("ru", include_str!("../i18n/ru.toml")),
    ("pt", include_str!("../i18n/pt.toml")),
    ("hi", include_str!("../i18n/hi.toml")),
];

#[derive(Debug, Clone)]
pub(crate) struct HydrolysisLocalizations(TranslationCatalog);

impl HydrolysisLocalizations {
    fn new() -> Self {
        let mut catalog = TranslationCatalog::new();
        for &(locale, source) in CATALOGS {
            catalog = catalog.add_toml(locale, source).unwrap_or_else(|error| {
                panic!("Hydrolysis built-in {locale} catalog must be valid TOML: {error}")
            });
        }
        Self(catalog)
    }
}

pub(crate) fn install(env: &mut Environment) {
    env.insert(HydrolysisLocalizations::new());
}

pub(crate) fn text(env: &Environment, key: &str) -> String {
    let localizations = env
        .get::<HydrolysisLocalizations>()
        .expect("Hydrolysis runner must install its built-in localizations");
    let locale = locale_binding(env).get();
    localizations
        .0
        .lookup_text(&locale, key)
        .unwrap_or_else(|| panic!("Hydrolysis built-in localization is missing key `{key}`"))
        .to_string()
}

#[cfg(test)]
mod tests {
    use nami::Binding;
    use waterui_locale::{Locale, locales};

    use super::*;

    const BUILTIN_KEYS: &[&str] = &[
        "copy",
        "cut",
        "paste",
        "select_all",
        "cancel",
        "ok",
        "back",
        "select_date",
        "date",
        "hour",
        "minute",
        "second",
        "opacity_50",
        "hdr_headroom",
        "alpha",
        "hdr",
        "color",
        "alpha_hdr",
        "Red",
        "Orange",
        "Amber",
        "Green",
        "Teal",
        "Cyan",
        "Blue",
        "Indigo",
        "Purple",
        "Pink",
        "Brown",
        "Grey",
    ];

    #[test]
    fn resolves_rtl_and_cjk_builtin_strings() {
        let locale = Binding::container(locales::AR);
        let mut env = Environment::new();
        env.insert(locale.clone());
        install(&mut env);
        assert_eq!(text(&env, "copy"), "نسخ");

        locale.set(locales::ZH_CN);
        assert_eq!(text(&env, "select_all"), "全选");

        locale.set("he".parse::<Locale>().expect("Hebrew locale must parse"));
        assert_eq!(text(&env, "back"), "חזרה");
    }

    #[test]
    fn every_builtin_catalog_defines_every_builtin_key() {
        for &(locale_name, source) in CATALOGS {
            let catalog = TranslationCatalog::new()
                .add_toml("en", source)
                .unwrap_or_else(|error| {
                    panic!("built-in locale {locale_name} is invalid: {error}")
                });
            for key in BUILTIN_KEYS {
                assert!(
                    catalog.lookup_text(&locales::EN, key).is_some(),
                    "Hydrolysis built-in {locale_name} catalog is missing key {key}"
                );
            }
        }
    }
}
