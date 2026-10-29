//! The application's light or dark appearance on Linux, as the colour-scheme
//! signal the winit host supplies to the theme tokens.
//!
//! winit reports no theme on X11 or Wayland, so the source of truth is the
//! XDG desktop portal's `org.freedesktop.portal.Settings` service: the
//! `org.freedesktop.appearance` `color-scheme` setting seeds the scheme
//! through `ReadOne`, and the `SettingChanged` signal — matched on the
//! daemon for exactly this namespace and key — writes each later value into
//! the same [`Binding`]. The framework default tokens are maps over the
//! scheme, so they re-resolve per slot and the views reading them repaint
//! with no tree rebuild.
//!
//! Per the portal spec an absent service and an unknown key both mean "no
//! preference", which reads as Light; any other unexpected reply — a wrong
//! variant type or a value outside the spec's `0..=2` — is a clear panic.
//! The client is one task on the runner's local executor, and zbus runs the
//! socket I/O on its own internal executor, so the event-loop thread never
//! blocks on D-Bus. The subscription dies with this value.
//!
//! `color_scheme_of` and `sync_portal_color_scheme` carry no platform
//! dependencies, so they compile — and are covered — in test builds on
//! every host; the zbus client lives in `portal`, Linux only.

use nami::Binding;
use waterui::theme::ColorScheme;

/// A portal `color-scheme` value mapped onto the scheme per the spec: `1`
/// is "prefer dark"; `0` ("no preference") and `2` ("prefer light") read as
/// Light.
///
/// # Panics
///
/// On a value outside the spec's `0..=2` — the spec names no fourth state.
fn color_scheme_of(value: u32) -> ColorScheme {
    match value {
        1 => ColorScheme::Dark,
        0 | 2 => ColorScheme::Light,
        _ => panic!(
            "hydrolysis Linux appearance: portal color-scheme {value} is outside the spec's 0–2"
        ),
    }
}

/// Writes the scheme `value` maps to into `scheme`, only when it moved.
///
/// Both the initial `ReadOne` and each `SettingChanged` delivery reach the
/// binding through this seam, so a report repeating the held scheme
/// notifies nothing.
fn sync_portal_color_scheme(scheme: &Binding<ColorScheme>, value: u32) {
    crate::theme::sync_color_scheme(scheme, color_scheme_of(value));
}

/// The D-Bus error names the portal spec calls "no preference": the portal
/// service is not on the bus, or the setting does not exist.
fn no_preference_error_name(name: &str) -> bool {
    matches!(
        name,
        "org.freedesktop.DBus.Error.ServiceUnknown"
            | "org.freedesktop.DBus.Error.NameHasNoOwner"
            | "org.freedesktop.portal.Error.NotFound"
    )
}

/// The XDG desktop portal client behind [`SystemAppearance`].
#[cfg(all(hydrolysis_winit, target_os = "linux"))]
mod portal {
    use std::cell::Cell;
    use std::rc::Rc;

    use executor_core::{LocalExecutor, async_task::AsyncTask};
    use futures_core::Stream as _;
    use nami::{Binding, Computed, Signal as _, SignalExt as _};
    use waterui::theme::ColorScheme;
    use zbus::zvariant::OwnedValue;

    use super::sync_portal_color_scheme;

    /// `org.freedesktop.portal.Settings`, served by `xdg-desktop-portal`.
    const SETTINGS_SERVICE: &str = "org.freedesktop.portal.Desktop";
    const SETTINGS_PATH: &str = "/org/freedesktop/portal/desktop";
    const SETTINGS_INTERFACE: &str = "org.freedesktop.portal.Settings";
    /// The portal namespace the colour scheme lives in.
    const APPEARANCE_NAMESPACE: &str = "org.freedesktop.appearance";
    /// The setting within it: `org.freedesktop.appearance` `color-scheme`.
    const COLOR_SCHEME_KEY: &str = "color-scheme";

    /// The portal's colour scheme as a signal, kept current for as long as
    /// this value lives.
    pub struct SystemAppearance {
        scheme: Binding<ColorScheme>,
        /// Flips once the initial `ReadOne` resolved — the real value or the
        /// spec's no-preference answer — which releases the runner's held
        /// first-window mount.
        initial_read: Rc<Cell<bool>>,
        /// The spawned subscription: `AsyncTask` cancels on drop, ending the
        /// portal watch with the runner.
        _watch: AsyncTask<()>,
    }

    impl core::fmt::Debug for SystemAppearance {
        fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
            f.debug_struct("SystemAppearance")
                .field("scheme", &self.scheme.snapshot())
                .finish_non_exhaustive()
        }
    }

    impl SystemAppearance {
        /// Starts following the portal's colour-scheme setting, with the
        /// client driven by `executor` — the runner's local executor, so
        /// every write lands on the event-loop thread.
        ///
        /// `wake` runs once the initial `ReadOne` resolves — the real value
        /// or the spec's no-preference answer for an absent bus, service or
        /// key — which is what releases the runner's held first mount.
        pub fn observe<E>(executor: &E, wake: impl Fn() + 'static) -> Self
        where
            E: LocalExecutor<Task<()> = AsyncTask<()>>,
        {
            let scheme = nami::binding(ColorScheme::Light);
            let initial_read = Rc::new(Cell::new(false));
            let watch = executor.spawn_local(watch(scheme.clone(), Rc::clone(&initial_read), wake));
            Self {
                scheme,
                initial_read,
                _watch: watch,
            }
        }

        /// The colour-scheme signal following the portal's setting.
        pub fn scheme(&self) -> Computed<ColorScheme> {
            self.scheme.computed()
        }

        /// Whether the initial `ReadOne` resolved. Until it has, the runner
        /// holds window mounts so the first frame already draws in the real
        /// scheme.
        pub fn initial_read_done(&self) -> bool {
            self.initial_read.get()
        }
    }

    /// Whether `error` is one of the cases the portal spec calls "no
    /// preference": the session bus cannot be reached at all, the portal
    /// service is not on it, or the setting does not exist.
    fn no_preference(error: &zbus::Error) -> bool {
        use super::no_preference_error_name;
        match error {
            // The session bus itself is unreachable: a bad or missing
            // address, socket I/O, or the connect itself.
            zbus::Error::Address(_) | zbus::Error::InputOutput(_) | zbus::Error::Connection(..) => {
                true
            }
            zbus::Error::MethodError(name, ..) => no_preference_error_name(name.as_str()),
            zbus::Error::FDO(error) => matches!(
                error.as_ref(),
                zbus::fdo::Error::ServiceUnknown(_) | zbus::fdo::Error::NameHasNoOwner(_)
            ),
            _ => false,
        }
    }

    /// Lets the errors the portal spec calls "no preference" through as
    /// `Err` for the caller to fold; every other error panics naming the
    /// step that failed.
    fn expected<T>(result: zbus::Result<T>, step: &str) -> zbus::Result<T> {
        result.map_err(|error| {
            assert!(
                no_preference(&error),
                "hydrolysis Linux appearance: {step} failed: {error}"
            );
            error
        })
    }

    /// The one task the whole client is: connect, subscribe, read once,
    /// then relay `SettingChanged` into the binding.
    #[expect(
        clippy::future_not_send,
        reason = "spawned with spawn_local on the event-loop thread; it holds the runner's Rc state"
    )]
    async fn watch(scheme: Binding<ColorScheme>, initial_read: Rc<Cell<bool>>, wake: impl Fn()) {
        // The match rule registers before the initial read so a change
        // landing between the two is queued by the daemon, not lost.
        let subscribed = async {
            let connection = expected(
                zbus::Connection::session().await,
                "connecting to the session bus",
            )?;
            let proxy = expected(
                zbus::Proxy::new(
                    &connection,
                    SETTINGS_SERVICE,
                    SETTINGS_PATH,
                    SETTINGS_INTERFACE,
                )
                .await,
                "building the portal Settings proxy",
            )?;
            let changes = expected(
                proxy
                    .receive_signal_with_args(
                        "SettingChanged",
                        &[(0, APPEARANCE_NAMESPACE), (1, COLOR_SCHEME_KEY)],
                    )
                    .await,
                "subscribing to SettingChanged",
            )?;
            // `ReadOne` exists only from version 2 of
            // `org.freedesktop.portal.Settings`; older portals answer
            // `Read`, the double-wrapped variant `portal_u32` peels.
            let version: u32 = expected(
                proxy.get_property("version").await,
                "reading the portal Settings version",
            )?;
            let reply = expected(
                if version >= 2 {
                    proxy
                        .call_method("ReadOne", &(APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY))
                        .await
                } else {
                    proxy
                        .call_method("Read", &(APPEARANCE_NAMESPACE, COLOR_SCHEME_KEY))
                        .await
                },
                "reading the portal colour scheme",
            )?;
            zbus::Result::Ok((proxy, changes, reply))
        }
        .await;
        let Ok((_proxy, mut changes, reply)) = subscribed else {
            // No session bus, no portal service or no such setting: the
            // spec's answer is "no preference" — Light, the value the
            // binding was seeded with.
            tracing::debug!(
                "hydrolysis Linux appearance: portal colour scheme unavailable; \
                 holding the spec's no-preference scheme"
            );
            initial_read.set(true);
            wake();
            return;
        };
        let value = reply
            .body()
            .deserialize::<OwnedValue>()
            .expect("hydrolysis Linux appearance: portal ReadOne reply is not a variant");
        sync_portal_color_scheme(&scheme, portal_u32(&value));
        initial_read.set(true);
        wake();
        while let Some(signal) =
            std::future::poll_fn(|cx| std::pin::Pin::new(&mut changes).poll_next(cx)).await
        {
            let args = signal
                .body()
                .deserialize::<(String, String, OwnedValue)>()
                .expect(
                    "hydrolysis Linux appearance: SettingChanged body is not the spec's (s, s, v)",
                );
            sync_portal_color_scheme(&scheme, portal_u32(&args.2));
        }
    }

    /// Reads the spec's `u` out of the variant the portal wraps it in,
    /// peeling nested variants: `ReadOne` and `SettingChanged` wrap once,
    /// while some services keep `Read`'s double wrap for `ReadOne` too.
    ///
    /// # Panics
    ///
    /// On anything that is not a `u` — the spec's `color-scheme` type.
    fn portal_u32(value: &OwnedValue) -> u32 {
        let mut leaf: &zbus::zvariant::Value<'static> = value;
        while let zbus::zvariant::Value::Value(inner) = leaf {
            leaf = inner.as_ref();
        }
        u32::try_from(leaf).unwrap_or_else(|_| {
            panic!(
                "hydrolysis Linux appearance: portal colour scheme {leaf:?} is not the spec's u32"
            )
        })
    }
}

#[cfg(all(hydrolysis_winit, target_os = "linux"))]
pub use portal::SystemAppearance;

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use nami::Signal as _;
    use waterui::theme::ColorScheme;
    use waterui_core::binding;

    use super::{color_scheme_of, no_preference_error_name, sync_portal_color_scheme};

    /// The spec's mapping: `1` is dark, `0` and `2` read as light, and a
    /// portal value moves the binding only when the scheme moved — a
    /// repeated report must not re-resolve every token slot.
    #[test]
    fn portal_values_write_the_scheme_only_on_a_move() {
        let scheme = binding(ColorScheme::Light);
        let notifications = Rc::new(Cell::new(0_u32));
        let _guard = scheme.watch({
            let notifications = Rc::clone(&notifications);
            move |_| notifications.set(notifications.get() + 1)
        });

        // "Prefer light" (2) and "no preference" (0) both hold Light —
        // writes that move nothing notify nothing.
        sync_portal_color_scheme(&scheme, 2);
        assert_eq!(scheme.snapshot(), ColorScheme::Light);
        assert_eq!(notifications.get(), 0);

        sync_portal_color_scheme(&scheme, 0);
        assert_eq!(notifications.get(), 0);

        // "Prefer dark" (1) is the one value that moves.
        sync_portal_color_scheme(&scheme, 1);
        assert_eq!(scheme.snapshot(), ColorScheme::Dark);
        assert_eq!(notifications.get(), 1);

        sync_portal_color_scheme(&scheme, 1);
        assert_eq!(notifications.get(), 1, "a repeated report writes nothing");

        sync_portal_color_scheme(&scheme, 0);
        assert_eq!(scheme.snapshot(), ColorScheme::Light);
        assert_eq!(notifications.get(), 2);
    }

    #[test]
    fn portal_values_map_per_the_spec() {
        assert_eq!(color_scheme_of(0), ColorScheme::Light);
        assert_eq!(color_scheme_of(1), ColorScheme::Dark);
        assert_eq!(color_scheme_of(2), ColorScheme::Light);
    }

    #[test]
    #[should_panic(expected = "outside the spec's 0–2")]
    fn a_portal_value_outside_the_spec_panics() {
        let _ = color_scheme_of(3);
    }

    /// Only the absent service and the unknown key are the spec's "no
    /// preference" — every other error name is a real failure.
    #[test]
    fn only_absent_service_and_unknown_key_are_no_preference() {
        for name in [
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.NameHasNoOwner",
            "org.freedesktop.portal.Error.NotFound",
        ] {
            assert!(no_preference_error_name(name), "{name}");
        }
        for name in [
            "org.freedesktop.DBus.Error.UnknownMethod",
            "org.freedesktop.DBus.Error.AccessDenied",
            "org.freedesktop.portal.Error.Cancelled",
            "org.freedesktop.portal.Error.InvalidArgument",
        ] {
            assert!(!no_preference_error_name(name), "{name}");
        }
    }
}
