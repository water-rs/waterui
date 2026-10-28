//! The application's light or dark appearance on macOS, as the colour-scheme
//! signal the winit host supplies to the theme tokens.
//!
//! The source of truth is `NSApp.effectiveAppearance`: the appearance the
//! user chose in System Settings unless the application overrides it. A
//! key-value observer on that property writes every change into one
//! [`Binding`], so the framework default tokens — maps over the scheme —
//! re-resolve per slot and the views reading them repaint, with no tree
//! rebuild. This is application-wide, independent of any window: a change
//! made while no window is open is in the binding when the next one mounts.
//!
//! The observation mirrors `cocoa-ui`'s `appkit::appearance` for the Apple
//! backend; that crate is an unpublished in-tree crate, so the published
//! Hydrolysis cannot depend on it.

use std::ffi::c_void;
use std::ptr;

use nami::{Binding, Computed, Signal, SignalExt as _};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication};
use objc2_foundation::{
    NSArray, NSDictionary, NSKeyValueObservingOptions, NSObject,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSString, ns_string,
};
use waterui::theme::ColorScheme;

/// The `NSApplication` property the observation follows.
fn key_path() -> &'static NSString {
    ns_string!("effectiveAppearance")
}

struct ObserverIvars {
    scheme: Binding<ColorScheme>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "WuiHydrolysisAppearanceObserver"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ObserverIvars]
    struct AppearanceObserver;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for AppearanceObserver {}

    impl AppearanceObserver {
        /// `effectiveAppearance` changed on the application: the binding
        /// takes the new scheme when it differs, so a change that keeps the
        /// scheme (a contrast or vibrancy variant) notifies nothing.
        // SAFETY: the signature is the one `NSKeyValueObserving` declares.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            _context: *mut c_void,
        ) {
            let application = object.expect("appearance KVO names the observed application");
            // SAFETY: the only registration is `effectiveAppearance` on
            // `NSApplication`, which answers it with an `NSAppearance`.
            let appearance: Retained<NSAppearance> =
                unsafe { msg_send![application, effectiveAppearance] };
            let scheme = color_scheme_of(&appearance);
            let binding = &self.ivars().scheme;
            if binding.snapshot() != scheme {
                binding.set(scheme);
            }
        }
    }
);

impl AppearanceObserver {
    fn new(mtm: MainThreadMarker, scheme: Binding<ColorScheme>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ObserverIvars { scheme });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// The application's appearance as a colour-scheme signal, kept current for
/// as long as this value lives.
pub struct SystemAppearance {
    scheme: Binding<ColorScheme>,
    application: Retained<NSApplication>,
    observer: Retained<AppearanceObserver>,
}

impl core::fmt::Debug for SystemAppearance {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SystemAppearance")
            .field("scheme", &self.scheme.snapshot())
            .finish_non_exhaustive()
    }
}

impl SystemAppearance {
    /// Reads the application's current appearance and observes its changes.
    ///
    /// # Panics
    ///
    /// Panics off the main thread: `NSApplication` is main-thread only.
    pub fn observe() -> Self {
        let mtm = MainThreadMarker::new()
            .expect("hydrolysis observes the application appearance on the main thread");
        let application = NSApplication::sharedApplication(mtm);
        let scheme = nami::binding(color_scheme_of(&application.effectiveAppearance()));
        let observer = AppearanceObserver::new(mtm, scheme.clone());
        // SAFETY: the observer implements the observation method; it is
        // registered for one key path and removed for that key path in
        // `Drop`, before the retained observer is released.
        unsafe {
            application.addObserver_forKeyPath_options_context(
                &observer,
                key_path(),
                NSKeyValueObservingOptions::New,
                ptr::null_mut(),
            );
        }
        Self {
            scheme,
            application,
            observer,
        }
    }

    /// The colour-scheme signal following the application's appearance.
    pub fn scheme(&self) -> Computed<ColorScheme> {
        self.scheme.computed()
    }
}

impl Drop for SystemAppearance {
    fn drop(&mut self) {
        // SAFETY: removes the registration `observe` made, for the same key
        // path, while the observer is still retained.
        unsafe {
            self.application
                .removeObserver_forKeyPath(&self.observer, key_path());
        }
    }
}

fn color_scheme_of(appearance: &NSAppearance) -> ColorScheme {
    // SAFETY: reads string constants `AppKit` owns for the process's
    // lifetime.
    let (dark, light) = unsafe { (NSAppearanceNameDarkAqua, NSAppearanceNameAqua) };
    let best = appearance.bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[dark, light]));
    // Dark exactly when the dark appearance is the closest match; an
    // appearance matching neither draws light.
    if best.as_deref() == Some(dark) {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}
