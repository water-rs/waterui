//! System locale observation on Linux through the XDG desktop portal's
//! `org.freedesktop.portal.Settings` `SettingChanged` signal.
//!
//! GNOME exposes the user's regional locale under `org.gnome.system.locale` /
//! `region` (for example `"de_DE.UTF-8"`), and KDE exposes the language list
//! under `org.kde.kdeglobals.Locale` / `Language` (for example
//! `"fr_FR:en_US"`). Subscribing to `SettingChanged` delivers changes into
//! `waterui_locale::regional` on the runner's local executor without polling.
//!
//! The pure parser [`parse_portal_locale_setting`] has no Linux-only
//! dependencies so it compiles and is unit-tested on every host.

use waterui_locale::regional::{refresh_system_locale, update_system_locale_tag};

/// GNOME's portal namespace for regional locale preferences.
const GNOME_LOCALE_NAMESPACE: &str = "org.gnome.system.locale";
/// GNOME's locale setting key inside [`GNOME_LOCALE_NAMESPACE`].
const GNOME_LOCALE_KEY: &str = "region";
/// KDE's portal namespace for locale preferences.
const KDE_LOCALE_NAMESPACE: &str = "org.kde.kdeglobals.Locale";
/// KDE's colon-separated preferred-language key inside [`KDE_LOCALE_NAMESPACE`].
const KDE_LOCALE_KEY: &str = "Language";

/// Parses a portal `SettingChanged` payload for a known locale namespace/key
/// into a canonicalized BCP 47 tag, or `None` if the event is unrelated or
/// carries no usable locale tag (such as `""`, `"C"`, or `"POSIX"`).
fn parse_portal_locale_setting(namespace: &str, key: &str, raw: &str) -> Option<String> {
    let primary = match (namespace, key) {
        (GNOME_LOCALE_NAMESPACE, GNOME_LOCALE_KEY) => raw.trim(),
        (KDE_LOCALE_NAMESPACE, KDE_LOCALE_KEY) => raw.split(':').next()?.trim(),
        _ => return None,
    };
    let without_modifier = primary.split('@').next().unwrap_or(primary);
    let without_codeset = without_modifier
        .split('.')
        .next()
        .unwrap_or(without_modifier)
        .trim();
    if without_codeset.is_empty()
        || without_codeset.eq_ignore_ascii_case("C")
        || without_codeset.eq_ignore_ascii_case("POSIX")
    {
        return None;
    }
    Some(without_codeset.replace('_', "-"))
}

/// Applies a portal locale setting update to `waterui_locale::regional`.
///
/// When `raw` parses to a valid BCP 47 tag it is published directly; when a
/// known locale key is cleared (for example reset to the system default) the
/// runtime re-reads the host environment via `refresh_system_locale`.
fn apply_portal_locale_change(namespace: &str, key: &str, raw: &str) {
    let is_locale_setting = matches!(
        (namespace, key),
        (GNOME_LOCALE_NAMESPACE, GNOME_LOCALE_KEY) | (KDE_LOCALE_NAMESPACE, KDE_LOCALE_KEY)
    );
    if !is_locale_setting {
        return;
    }
    if let Some(tag) = parse_portal_locale_setting(namespace, key, raw) {
        if let Err(error) = update_system_locale_tag(&tag) {
            tracing::warn!(
                namespace,
                key,
                tag,
                %error,
                "hydrolysis Linux locale: portal reported an invalid locale tag; refreshing from system"
            );
            let _ = refresh_system_locale();
        }
    } else {
        let _ = refresh_system_locale();
    }
}

#[cfg(all(hydrolysis_winit, target_os = "linux"))]
mod portal {
    use core::fmt;
    use core::future::poll_fn;
    use core::pin::Pin;

    use executor_core::{LocalExecutor, async_task::AsyncTask};
    use futures_core::Stream as _;
    use zbus::zvariant::{OwnedValue, Value};
    use zbus::{Connection, Proxy};

    use super::{
        GNOME_LOCALE_KEY, GNOME_LOCALE_NAMESPACE, KDE_LOCALE_KEY, KDE_LOCALE_NAMESPACE,
        apply_portal_locale_change,
    };

    const SETTINGS_SERVICE: &str = "org.freedesktop.portal.Desktop";
    const SETTINGS_PATH: &str = "/org/freedesktop/portal/desktop";
    const SETTINGS_INTERFACE: &str = "org.freedesktop.portal.Settings";

    /// Keeps the XDG desktop portal locale subscription alive for the runner's lifetime.
    pub struct SystemLocale {
        _watch: AsyncTask<()>,
    }

    impl fmt::Debug for SystemLocale {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("SystemLocale").finish_non_exhaustive()
        }
    }

    impl SystemLocale {
        /// Starts observing locale changes from `org.freedesktop.portal.Settings`
        /// on the runner's local executor.
        pub fn observe<E>(executor: &E) -> Self
        where
            E: LocalExecutor<Task<()> = AsyncTask<()>>,
        {
            Self {
                _watch: executor.spawn_local(watch()),
            }
        }
    }

    fn peel_str(value: &OwnedValue) -> Option<&str> {
        let mut leaf: &Value<'static> = value;
        while let Value::Value(inner) = leaf {
            leaf = inner;
        }
        match leaf {
            Value::Str(text) => Some(text.as_str()),
            _ => None,
        }
    }

    async fn watch() {
        let connection = match Connection::session().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "hydrolysis Linux locale: session bus unavailable; skipping portal locale watch"
                );
                return;
            }
        };
        let proxy = match Proxy::new(
            &connection,
            SETTINGS_SERVICE,
            SETTINGS_PATH,
            SETTINGS_INTERFACE,
        )
        .await
        {
            Ok(proxy) => proxy,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "hydrolysis Linux locale: portal Settings proxy unavailable"
                );
                return;
            }
        };
        let mut changes = match proxy.receive_signal("SettingChanged").await {
            Ok(stream) => stream,
            Err(error) => {
                tracing::debug!(
                    %error,
                    "hydrolysis Linux locale: failed to subscribe to portal SettingChanged"
                );
                return;
            }
        };

        while let Some(signal) = poll_fn(|cx| Pin::new(&mut changes).poll_next(cx)).await {
            let Ok((namespace, key, value)): zbus::Result<(String, String, OwnedValue)> =
                signal.body().deserialize()
            else {
                continue;
            };
            if matches!(
                (namespace.as_str(), key.as_str()),
                (GNOME_LOCALE_NAMESPACE, GNOME_LOCALE_KEY) | (KDE_LOCALE_NAMESPACE, KDE_LOCALE_KEY)
            ) {
                let raw = peel_str(&value).unwrap_or("");
                apply_portal_locale_change(&namespace, &key, raw);
            }
        }
    }
}

#[cfg(all(hydrolysis_winit, target_os = "linux"))]
pub use portal::SystemLocale;

#[cfg(test)]
mod tests {
    use waterui_locale::regional::current_settings;

    use super::*;

    #[test]
    fn parses_gnome_and_kde_portal_locale_settings() {
        assert_eq!(
            parse_portal_locale_setting("org.gnome.system.locale", "region", "de_DE.UTF-8"),
            Some("de-DE".to_string())
        );
        assert_eq!(
            parse_portal_locale_setting("org.gnome.system.locale", "region", "zh_CN.UTF-8@hans"),
            Some("zh-CN".to_string())
        );
        assert_eq!(
            parse_portal_locale_setting("org.kde.kdeglobals.Locale", "Language", "ja_JP:en_US"),
            Some("ja-JP".to_string())
        );
        assert_eq!(
            parse_portal_locale_setting("org.kde.kdeglobals.Locale", "Language", "ko_KR"),
            Some("ko-KR".to_string())
        );
        assert_eq!(
            parse_portal_locale_setting("org.gnome.system.locale", "region", "C.UTF-8"),
            None
        );
        assert_eq!(
            parse_portal_locale_setting("org.freedesktop.appearance", "color-scheme", "1"),
            None
        );

        apply_portal_locale_change("org.gnome.system.locale", "region", "ja_JP.UTF-8");
        assert_eq!(current_settings().locale_tag(), "ja-JP");
    }
}
