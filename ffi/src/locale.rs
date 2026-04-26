//! # Locale FFI
//!
//! This module provides FFI bindings for locale/i18n support in WaterUI.
//!
//! ## Overview
//!
//! Native backends (iOS, Android) should detect the system locale and install it:
//!
//! ```c
//! // Get system locale identifier (e.g., "en-US", "zh-Hans-CN", "ja-JP")
//! const char* locale_id = get_system_locale();
//!
//! // Install it into the environment
//! waterui_env_install_locale_string(env, locale_id);
//! ```
//!
//! ## Supported Locales
//!
//! WaterUI supports all BCP 47 locale identifiers including:
//! - English: "en", "en-US", "en-GB"
//! - Chinese: "zh", "zh-CN", "zh-TW", "zh-Hans", "zh-Hant"
//! - Japanese: "ja", "ja-JP"
//! - Korean: "ko", "ko-KR"
//! - German: "de", "de-DE", "de-AT", "de-CH"
//! - French: "fr", "fr-FR", "fr-CA"
//! - Spanish: "es", "es-ES", "es-MX"
//! - Russian: "ru", "ru-RU"
//! - And many more...

use core::ffi::c_char;
use core::str::FromStr;

use waterui::Str;
use waterui_locale::{Locale, regional};

use crate::{IntoFFI, WuiEnv};

/// Locale enum for common locales (for convenience).
///
/// For locales not in this enum, use `waterui_env_install_locale_string()`.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WuiLocale {
    /// English (US)
    EnUs = 0,
    /// English (UK)
    EnGb = 1,
    /// Chinese (Simplified, China)
    ZhCn = 2,
    /// Chinese (Traditional, Taiwan)
    ZhTw = 3,
    /// Chinese (Traditional, Hong Kong)
    ZhHk = 4,
    /// Japanese
    Ja = 5,
    /// Korean
    Ko = 6,
    /// German
    De = 7,
    /// French
    Fr = 8,
    /// Spanish
    Es = 9,
    /// Russian
    Ru = 10,
    /// Arabic
    Ar = 11,
}

impl From<WuiLocale> for Locale {
    fn from(value: WuiLocale) -> Self {
        use waterui_locale::locales;
        match value {
            WuiLocale::EnUs => locales::EN_US,
            WuiLocale::EnGb => locales::EN_GB,
            WuiLocale::ZhCn => locales::ZH_CN,
            WuiLocale::ZhTw => locales::ZH_TW,
            WuiLocale::ZhHk => locales::ZH_HK,
            WuiLocale::Ja => locales::JA,
            WuiLocale::Ko => locales::KO,
            WuiLocale::De => locales::DE,
            WuiLocale::Fr => locales::FR,
            WuiLocale::Es => locales::ES,
            WuiLocale::Ru => locales::RU,
            WuiLocale::Ar => locales::AR,
        }
    }
}

fn parse_locale(locale_id: &str) -> Locale {
    Locale::from_str(locale_id).unwrap_or_else(|error| {
        panic!("Invalid locale '{locale_id}': {error}");
    })
}

fn install_locale_value(env: &mut WuiEnv, locale: Locale) {
    env.0.insert(locale.clone());
    regional::set_locale_tag(locale.canonical_tag())
        .expect("locale inserted into environment must be valid");
    env.0
        .insert(regional::current_settings().with_locale(&locale));
}

fn current_locale(env: &WuiEnv) -> Locale {
    if let Some(locale) = env.0.get::<Locale>().cloned() {
        return locale;
    }
    if let Some(context) = env.0.get::<regional::RegionalContext>() {
        return context.locale().clone();
    }
    parse_locale(regional::current_settings().locale_tag())
}

/// Installs a locale into the environment using a predefined locale enum.
///
/// This installs a `Locale` snapshot into the environment and publishes it
/// to the shared regional runtime context.
///
/// # Safety
/// - `env` must be a valid pointer from `waterui_init()` or `waterui_env_new()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_locale(env: *mut WuiEnv, locale: WuiLocale) {
    let env = unsafe { crate::expect_non_null_mut(env, "waterui_env_install_locale", "env") };
    let rust_locale: Locale = locale.into();
    install_locale_value(env, rust_locale.clone());
    tracing::debug!("Installed locale {:?}", rust_locale);
}

/// Installs a locale into the environment using a BCP 47 locale string.
///
/// This is more flexible than `waterui_env_install_locale()` as it accepts
/// any valid BCP 47 locale identifier (e.g., "en-US", "zh-Hans-CN", "ja-JP").
///
/// If the locale string is invalid, falls back to English ("en").
///
/// This installs a `Locale` snapshot into the environment and publishes it
/// to the shared regional runtime context.
///
/// # Safety
/// - `env` must be a valid pointer from `waterui_init()` or `waterui_env_new()`.
/// - `locale_str` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_locale_string(
    env: *mut WuiEnv,
    locale_str: *const c_char,
) {
    let env =
        unsafe { crate::expect_non_null_mut(env, "waterui_env_install_locale_string", "env") };
    let locale_str = unsafe {
        crate::expect_non_null(
            locale_str,
            "waterui_env_install_locale_string",
            "locale_str",
        )
    };

    // Convert C string to Rust &str
    let c_str = unsafe { core::ffi::CStr::from_ptr(locale_str) };
    let locale_id = unsafe { core::str::from_utf8_unchecked(c_str.to_bytes()) };
    let rust_locale = parse_locale(locale_id);
    install_locale_value(env, rust_locale.clone());
    tracing::debug!("Installed locale from string {:?}", rust_locale);
}

/// Gets the current locale from the environment.
///
/// Returns the locale as a WuiLocale enum. If the locale doesn't match
/// any predefined enum value, returns `WuiLocale::EnUs` as default.
///
/// # Safety
/// - `env` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_get_locale(env: *const WuiEnv) -> WuiLocale {
    let env = unsafe { crate::expect_non_null(env, "waterui_env_get_locale", "env") };
    let locale = current_locale(env);
    let lang = locale.language.as_str();

    match lang {
        "en" => match locale.region.as_ref().map(|r| r.as_str()) {
            Some("GB") => WuiLocale::EnGb,
            _ => WuiLocale::EnUs,
        },
        "zh" => {
            // Check for region/script
            if let Some(region) = locale.region {
                match region.as_str() {
                    "TW" => WuiLocale::ZhTw,
                    "HK" => WuiLocale::ZhHk,
                    _ => WuiLocale::ZhCn,
                }
            } else if let Some(script) = locale.script {
                match script.as_str() {
                    "Hant" => WuiLocale::ZhTw,
                    _ => WuiLocale::ZhCn,
                }
            } else {
                WuiLocale::ZhCn
            }
        }
        "ja" => WuiLocale::Ja,
        "ko" => WuiLocale::Ko,
        "de" => WuiLocale::De,
        "fr" => WuiLocale::Fr,
        "es" => WuiLocale::Es,
        "ru" => WuiLocale::Ru,
        "ar" => WuiLocale::Ar,
        _ => panic!(
            "waterui_env_get_locale: unsupported language '{}' for WuiLocale; use waterui_env_get_locale_tag for lossless locale",
            lang
        ),
    }
}

/// Gets the current locale from the environment as a canonical BCP 47 string.
///
/// This is a lossless alternative to `waterui_env_get_locale()`.
///
/// # Safety
/// - `env` must be a valid pointer or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_get_locale_tag(env: *const WuiEnv) -> crate::WuiStr {
    let env = unsafe { crate::expect_non_null(env, "waterui_env_get_locale_tag", "env") };
    let locale = current_locale(env);
    Str::from(locale.canonical_tag()).into_ffi()
}

#[cfg(test)]
mod tests {
    use std::boxed::Box;
    use std::ffi::CString;

    use super::*;

    #[test]
    fn install_locale_writes_environment_snapshot() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            waterui_env_install_locale(env_ptr, WuiLocale::EnGb);
            let env = &*env_ptr;

            let locale = env.0.get::<Locale>().expect("Locale should be installed");
            assert_eq!(locale.language.as_str(), "en");
            assert_eq!(locale.region.as_ref().map(|r| r.as_str()), Some("GB"));
            let settings = env
                .0
                .get::<regional::RegionalContext>()
                .expect("RegionalContext should be installed");
            assert_eq!(settings.locale_tag(), "en-GB");
            assert!(!settings.timezone().is_empty());

            drop(Box::from_raw(env_ptr));
        }
    }

    #[test]
    fn install_locale_updates_runtime_context() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            waterui_env_install_locale(env_ptr, WuiLocale::EnUs);
            waterui_env_install_locale(env_ptr, WuiLocale::EnGb);
            assert_eq!(regional::current_settings().locale_tag(), "en-GB");

            drop(Box::from_raw(env_ptr));
        }
    }

    #[test]
    fn get_locale_preserves_english_region() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            waterui_env_install_locale(env_ptr, WuiLocale::EnGb);
            assert_eq!(waterui_env_get_locale(env_ptr), WuiLocale::EnGb);
            drop(Box::from_raw(env_ptr));
        }
    }

    #[test]
    fn install_locale_string_updates_environment_snapshot() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            let locale = CString::new("en-GB").expect("valid c string");
            waterui_env_install_locale_string(env_ptr, locale.as_ptr());

            let env = &*env_ptr;
            let locale_value = env.0.get::<Locale>().expect("Locale should be installed");
            assert_eq!(locale_value.language.as_str(), "en");
            assert_eq!(locale_value.region.as_ref().map(|r| r.as_str()), Some("GB"));
            let settings = env
                .0
                .get::<regional::RegionalContext>()
                .expect("RegionalContext should be installed");
            assert_eq!(settings.locale_tag(), "en-GB");
            assert_eq!(waterui_env_get_locale(env_ptr), WuiLocale::EnGb);

            drop(Box::from_raw(env_ptr));
        }
    }

    #[test]
    fn get_locale_tag_returns_lossless_bcp47() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            let locale = CString::new("en-GB-u-hc-h23").expect("valid c string");
            waterui_env_install_locale_string(env_ptr, locale.as_ptr());

            let tag = waterui_env_get_locale_tag(env_ptr);
            assert_eq!(tag.as_str(), "en-GB-u-hc-h23");

            drop(Box::from_raw(env_ptr));
        }
    }
}
