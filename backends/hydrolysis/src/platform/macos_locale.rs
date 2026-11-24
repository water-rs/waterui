//! System locale observation on macOS for the winit host.
//!
//! Subscribes to `NSCurrentLocaleDidChangeNotification` on
//! `NSNotificationCenter.defaultCenter` and calls
//! [`waterui_locale::regional::refresh_system_locale`] whenever macOS signals
//! a language or regional setting change, so locale-sensitive views update
//! without polling.

use core::fmt;

use objc2::rc::Retained;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_foundation::{
    NSCurrentLocaleDidChangeNotification, NSNotification, NSNotificationCenter, NSObject,
    NSObjectProtocol,
};
use waterui_locale::regional::refresh_system_locale;

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "WuiHydrolysisLocaleObserver"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ()]
    struct LocaleObserver;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for LocaleObserver {}

    impl LocaleObserver {
        // SAFETY: selector takes one `NSNotification*` argument per
        // `NSNotificationCenter`'s `addObserver:selector:name:object:` contract.
        #[unsafe(method(localeDidChange:))]
        fn locale_did_change(&self, _notification: Option<&NSNotification>) {
            let _ = refresh_system_locale();
        }
    }
);

impl LocaleObserver {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Keeps the `NSCurrentLocaleDidChangeNotification` observer registered for
/// as long as this value lives.
pub struct SystemLocale {
    observer: Retained<LocaleObserver>,
}

impl fmt::Debug for SystemLocale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SystemLocale").finish_non_exhaustive()
    }
}

impl SystemLocale {
    /// Registers the notification observer on the main thread.
    ///
    /// # Panics
    ///
    /// Panics if called off the main thread.
    #[must_use]
    pub fn observe() -> Self {
        let mtm = MainThreadMarker::new().expect(
            "the winit event loop exists only on the main thread, and the runner observes locale changes after building it",
        );
        let observer = LocaleObserver::new(mtm);
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: `observer` implements `localeDidChange:` and `Drop` removes
        // it from `defaultCenter` before it is released; `NSCurrentLocaleDidChangeNotification`
        // is a static Foundation constant alive for the process.
        unsafe {
            center.addObserver_selector_name_object(
                &observer,
                sel!(localeDidChange:),
                Some(NSCurrentLocaleDidChangeNotification),
                None,
            );
        }
        Self { observer }
    }
}

impl Drop for SystemLocale {
    fn drop(&mut self) {
        let center = NSNotificationCenter::defaultCenter();
        // SAFETY: removes the registration installed in `observe` while
        // `self.observer` is still retained.
        unsafe {
            center.removeObserver(&self.observer);
        }
    }
}
