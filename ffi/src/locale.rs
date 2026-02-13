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

use nami::SignalExt;
use waterui::{Binding, Computed, Signal};
use waterui_locale::Locale;

use crate::WuiEnv;

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

#[derive(Clone)]
struct LocaleSignalState {
    binding: Binding<Locale>,
}

impl LocaleSignalState {
    fn new(binding: Binding<Locale>) -> Self {
        Self { binding }
    }
}

fn parse_locale_or_default(locale_id: &str) -> Locale {
    match Locale::from_str(locale_id) {
        Ok(locale) => locale,
        Err(_) => {
            tracing::warn!("Invalid locale '{}', falling back to English", locale_id);
            waterui_locale::locales::EN
        }
    }
}

fn install_locale_value(env: &mut WuiEnv, locale: Locale) {
    let binding = if let Some(state) = env.0.get::<LocaleSignalState>() {
        state.binding.set(locale.clone());
        state.binding.clone()
    } else {
        let binding = Binding::container(locale.clone());
        env.0.insert(LocaleSignalState::new(binding.clone()));
        binding
    };

    // Keep both a static value and a reactive signal in the environment.
    env.0.insert(locale);
    env.0.insert(binding.computed());
}

fn current_locale(env: &WuiEnv) -> Locale {
    if let Some(locale_signal) = env.0.get::<Computed<Locale>>() {
        return locale_signal.get();
    }
    env.0
        .get::<Locale>()
        .cloned()
        .unwrap_or(waterui_locale::locales::EN_US)
}

/// Installs a locale into the environment using a predefined locale enum.
///
/// This installs both:
/// - `Locale` (snapshot value)
/// - `Computed<Locale>` (reactive signal for dynamic locale updates)
///
/// # Safety
/// - `env` must be a valid pointer from `waterui_init()` or `waterui_env_new()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_locale(env: *mut WuiEnv, locale: WuiLocale) {
    if env.is_null() {
        return;
    }
    let env = unsafe { &mut *env };
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
/// This installs both:
/// - `Locale` (snapshot value)
/// - `Computed<Locale>` (reactive signal for dynamic locale updates)
///
/// # Safety
/// - `env` must be a valid pointer from `waterui_init()` or `waterui_env_new()`.
/// - `locale_str` must be a valid null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_locale_string(
    env: *mut WuiEnv,
    locale_str: *const c_char,
) {
    if env.is_null() || locale_str.is_null() {
        return;
    }

    let env = unsafe { &mut *env };

    // Convert C string to Rust &str
    let c_str = unsafe { core::ffi::CStr::from_ptr(locale_str) };
    let locale_id = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => {
            tracing::warn!("Invalid UTF-8 in locale string, falling back to English");
            "en"
        }
    };

    let rust_locale = parse_locale_or_default(locale_id);
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
    if env.is_null() {
        return WuiLocale::EnUs;
    }

    let env = unsafe { &*env };
    let locale = current_locale(env);
    let lang = locale.0.language.as_str();

    match lang {
        "en" => match locale.0.region.as_ref().map(|r| r.as_str()) {
            Some("GB") => WuiLocale::EnGb,
            _ => WuiLocale::EnUs,
        },
        "zh" => {
            // Check for region/script
            if let Some(region) = locale.0.region {
                match region.as_str() {
                    "TW" => WuiLocale::ZhTw,
                    "HK" => WuiLocale::ZhHk,
                    _ => WuiLocale::ZhCn,
                }
            } else if let Some(script) = locale.0.script {
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
        _ => WuiLocale::EnUs,
    }
}

#[cfg(test)]
mod tests {
    use std::boxed::Box;
    use std::ffi::CString;

    use super::*;

    #[test]
    fn install_locale_injects_reactive_computed() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            waterui_env_install_locale(env_ptr, WuiLocale::EnGb);
            let env = &*env_ptr;

            let locale = env.0.get::<Locale>().expect("Locale should be installed");
            assert_eq!(locale.language.as_str(), "en");
            assert_eq!(locale.region.as_ref().map(|r| r.as_str()), Some("GB"));

            let locale_signal = env
                .0
                .get::<Computed<Locale>>()
                .expect("Computed<Locale> should be installed");
            let locale_value = locale_signal.get();
            assert_eq!(locale_value.language.as_str(), "en");
            assert_eq!(locale_value.region.as_ref().map(|r| r.as_str()), Some("GB"));

            drop(Box::from_raw(env_ptr));
        }
    }

    #[test]
    fn install_locale_updates_existing_signal() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            waterui_env_install_locale(env_ptr, WuiLocale::EnUs);
            let env = &*env_ptr;
            let locale_signal = env
                .0
                .get::<Computed<Locale>>()
                .expect("Computed<Locale> should be installed")
                .clone();

            waterui_env_install_locale(env_ptr, WuiLocale::EnGb);
            let locale_value = locale_signal.get();
            assert_eq!(locale_value.language.as_str(), "en");
            assert_eq!(locale_value.region.as_ref().map(|r| r.as_str()), Some("GB"));

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
    fn install_locale_string_updates_reactive_signal() {
        let env_ptr = crate::waterui_env_new();
        unsafe {
            let locale = CString::new("en-GB").expect("valid c string");
            waterui_env_install_locale_string(env_ptr, locale.as_ptr());

            let env = &*env_ptr;
            let locale_signal = env
                .0
                .get::<Computed<Locale>>()
                .expect("Computed<Locale> should be installed");
            let locale_value = locale_signal.get();
            assert_eq!(locale_value.language.as_str(), "en");
            assert_eq!(locale_value.region.as_ref().map(|r| r.as_str()), Some("GB"));
            assert_eq!(waterui_env_get_locale(env_ptr), WuiLocale::EnGb);

            drop(Box::from_raw(env_ptr));
        }
    }
}
