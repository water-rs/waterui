//! # Locale FFI
//!
//! This module provides FFI bindings for locale/i18n support in `WaterUI`.
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
//! waterui_env_install_locale_tag(env, locale_id);
//! ```
//!
//! ## Supported Locales
//!
//! `WaterUI` supports all BCP 47 locale identifiers including:
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

use nami::{Binding, Container};
use waterui_locale::{Locale, regional};

use crate::WuiEnv;

fn parse_locale(locale_id: &str) -> Locale {
    Locale::from_str(locale_id).unwrap_or_else(|error| {
        panic!("Invalid locale '{locale_id}': {error}");
    })
}

fn install_locale_value(env: &mut WuiEnv, locale: Locale) {
    let binding = env.0.get::<Binding<Locale>>().cloned();
    regional::set_locale_tag(locale.canonical_tag())
        .expect("parsed locale tag must remain valid when published");
    let context = regional::current_settings().with_locale(&locale);
    env.0.insert(locale.clone());
    env.0.insert(context);
    if let Some(binding) = binding {
        binding.set(locale);
    } else {
        env.0.insert(Binding::custom(Container::new(locale)));
    }
}

pub(crate) fn install_locale_tag(env: &mut WuiEnv, locale_tag: &str) {
    install_locale_value(env, parse_locale(locale_tag));
}

/// Installs a locale into the environment using a BCP 47 locale string.
///
/// This installs a reactive locale binding into the environment and publishes
/// the same value to the shared regional runtime. Invalid tags fail immediately.
///
/// # Safety
/// - `env` must be a valid pointer returned by `waterui_init()`.
/// - `locale_str` must be a valid null-terminated UTF-8 string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn waterui_env_install_locale_tag(
    env: *mut WuiEnv,
    locale_tag: *const c_char,
) {
    let env = unsafe { crate::borrow_ffi_mut(env) };
    let c_str = unsafe { core::ffi::CStr::from_ptr(locale_tag) };
    let locale_id = unsafe { core::str::from_utf8_unchecked(c_str.to_bytes()) };
    install_locale_tag(env, locale_id);
}

#[cfg(feature = "android-jni")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_waterui_android_ffi_WatcherJni_envInstallLocaleTag<'local>(
    mut jni_env: crate::jni::JNIEnv<'local>,
    _class: crate::jni::JClass<'local>,
    env_ptr: crate::jni::jlong,
    locale_tag: crate::jni::jni::objects::JString<'local>,
) {
    crate::jni::with_env(&mut jni_env, |jni_env| {
        let locale_tag = crate::jni::convert::string_from_java(jni_env, &locale_tag);
        let env = unsafe { crate::borrow_ffi_mut(env_ptr as *mut WuiEnv) };
        install_locale_tag(env, &locale_tag);
    });
}

#[cfg(test)]
mod tests {
    use std::boxed::Box;
    use std::cell::RefCell;
    use std::ffi::CString;
    use std::rc::Rc;

    use nami::Signal;

    use super::*;

    #[test]
    fn install_locale_reuses_reactive_environment_binding() {
        let env_ptr = Box::into_raw(Box::new(WuiEnv(waterui::Environment::new())));
        unsafe {
            let initial = CString::new("en-GB").expect("valid locale C string");
            waterui_env_install_locale_tag(env_ptr, initial.as_ptr());

            let env = &*env_ptr;
            let binding = env
                .0
                .get::<Binding<Locale>>()
                .expect("locale binding should be installed")
                .clone();
            assert_eq!(binding.get().canonical_tag(), "en-GB");

            let observed = Rc::new(RefCell::new(None));
            let callback_observed = Rc::clone(&observed);
            let _guard = binding.watch(move |ctx| {
                callback_observed
                    .borrow_mut()
                    .replace(ctx.into_value().canonical_tag());
            });

            let updated = CString::new("ja-JP").expect("valid locale C string");
            waterui_env_install_locale_tag(env_ptr, updated.as_ptr());
            assert_eq!(binding.get().canonical_tag(), "ja-JP");
            assert_eq!(observed.borrow().as_deref(), Some("ja-JP"));

            drop(Box::from_raw(env_ptr));
        }
    }

    #[test]
    #[should_panic(expected = "Invalid locale")]
    fn invalid_locale_tag_fails_during_parsing() {
        let _ = parse_locale("en--US");
    }
}
