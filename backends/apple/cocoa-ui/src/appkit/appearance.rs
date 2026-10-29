//! The application's light or dark appearance, and its changes.
//!
//! # Safety
//!
//! The `unsafe` here reads `AppKit`'s appearance-name constants, and
//! registers and removes a key-value observer. The observer is an object of a
//! class defined here, whose observation method has the signature
//! `NSKeyValueObserving` declares; it is registered on the application for one
//! key path and removed for that key path before it is released, because
//! [`ColorSchemeObservation`] owns it and unregisters it on drop.

use std::ffi::c_void;
use std::fmt;
use std::ptr;

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSView,
};
use objc2_foundation::{
    NSArray, NSDictionary, NSKeyValueObservingOptions, NSObject,
    NSObjectNSKeyValueObserverRegistration, NSObjectProtocol, NSString, ns_string,
};

use super::application::Application;
use crate::callback::guarded;
use crate::color_scheme::ColorScheme;

/// The application property whose changes the observation follows.
fn key_path() -> &'static NSString {
    ns_string!("effectiveAppearance")
}

impl Application {
    /// Whether the application currently draws light or dark.
    ///
    /// This is the appearance the user chose in System Settings unless the
    /// application overrides it.
    #[must_use]
    pub fn color_scheme(&self) -> ColorScheme {
        color_scheme_of(&self.native().effectiveAppearance())
    }

    /// Calls `handler` with the new scheme every time the application's
    /// appearance changes, until the returned guard is dropped.
    ///
    /// A panic in `handler` aborts the process (see the
    /// [crate documentation](crate)).
    pub fn observe_color_scheme(
        &self,
        handler: impl Fn(ColorScheme) + 'static,
    ) -> ColorSchemeObservation {
        observe_target(self.native(), handler)
    }
}

/// Whether `view`'s effective appearance draws light or dark.
#[must_use]
pub fn effective_color_scheme(view: &NSView) -> ColorScheme {
    color_scheme_of(&view.effectiveAppearance())
}

/// Calls `handler` with the new scheme every time `view`'s
/// `effectiveAppearance` changes.
///
/// That is when its own or an ancestor's appearance changes, or when it
/// moves to a window drawing differently — until the returned guard is
/// dropped. The guard keeps the view retained so a live view always
/// unregisters.
///
/// A panic in `handler` aborts the process (see the
/// [crate documentation](crate)).
pub fn observe_effective_appearance(
    view: &NSView,
    handler: impl Fn(ColorScheme) + 'static,
) -> ColorSchemeObservation {
    observe_target(view, handler)
}

/// `effectiveAppearance` KVO on any `NSObject` answering it — the
/// application, or a view through [`observe_effective_appearance`].
fn observe_target(
    target: &NSObject,
    handler: impl Fn(ColorScheme) + 'static,
) -> ColorSchemeObservation {
    let retained = target.retain();
    let observer = Observer::new(
        MainThreadMarker::new().expect("appearance observation runs on the main thread"),
        Box::new(handler),
    );
    // SAFETY: see the module safety note.
    unsafe {
        retained.addObserver_forKeyPath_options_context(
            &observer,
            key_path(),
            NSKeyValueObservingOptions::New,
            ptr::null_mut(),
        );
    }
    ColorSchemeObservation {
        observed: retained,
        observer,
    }
}

/// Keeps a color-scheme observer registered; dropping it removes the
/// observer.
#[must_use = "the observer is removed as soon as this guard is dropped"]
pub struct ColorSchemeObservation {
    observed: Retained<NSObject>,
    observer: Retained<Observer>,
}

impl fmt::Debug for ColorSchemeObservation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ColorSchemeObservation")
            .finish_non_exhaustive()
    }
}

impl Drop for ColorSchemeObservation {
    fn drop(&mut self) {
        // SAFETY: see the module safety note.
        unsafe {
            self.observed
                .removeObserver_forKeyPath(&self.observer, key_path());
        };
    }
}

pub fn color_scheme_of(appearance: &NSAppearance) -> ColorScheme {
    // SAFETY: reads string constants `AppKit` owns for the process's
    // lifetime.
    let (dark, light) = unsafe { (NSAppearanceNameDarkAqua, NSAppearanceNameAqua) };
    let best = appearance.bestMatchFromAppearancesWithNames(&NSArray::from_slice(&[dark, light]));
    scheme_for_best_match(best.as_deref(), dark)
}

/// Dark exactly when the dark appearance is the closest match; an appearance
/// matching neither (a high-contrast or vibrant variant with no counterpart)
/// is drawn light.
fn scheme_for_best_match(best: Option<&NSString>, dark: &NSString) -> ColorScheme {
    if best == Some(dark) {
        ColorScheme::Dark
    } else {
        ColorScheme::Light
    }
}

struct ObserverIvars {
    handler: Box<dyn Fn(ColorScheme)>,
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "CocoaUiAppearanceObserver"]
    #[thread_kind = MainThreadOnly]
    #[ivars = ObserverIvars]
    struct Observer;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSObject` subclass.
    unsafe impl NSObjectProtocol for Observer {}

    impl Observer {
        // SAFETY: see the module safety note.
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: Option<&NSString>,
            _object: Option<&AnyObject>,
            _change: Option<&NSDictionary<NSString, AnyObject>>,
            _context: *mut c_void,
        ) {
            guarded("effectiveAppearance observer", || {
                let target = _object
                    .expect("appearance KVO must name the observed object");
                // SAFETY: the only registered key path is
                // `effectiveAppearance`, answered by `NSApplication` and
                // `NSView` — the two callers — with an `NSAppearance`.
                let appearance: Retained<NSAppearance> =
                    unsafe { msg_send![target, effectiveAppearance] };
                (self.ivars().handler)(color_scheme_of(&appearance));
            });
        }
    }
);

impl Observer {
    fn new(mtm: MainThreadMarker, handler: Box<dyn Fn(ColorScheme)>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ObserverIvars { handler });
        // SAFETY: `init` is `NSObject`'s designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use objc2_app_kit::{NSAppearanceNameAqua, NSAppearanceNameDarkAqua};

    use super::scheme_for_best_match;
    use crate::color_scheme::ColorScheme;

    #[test]
    fn only_the_dark_match_is_dark() {
        // SAFETY: reads string constants `AppKit` owns for the process's
        // lifetime.
        let (dark, light) = unsafe { (NSAppearanceNameDarkAqua, NSAppearanceNameAqua) };
        assert_eq!(scheme_for_best_match(Some(dark), dark), ColorScheme::Dark);
        assert_eq!(scheme_for_best_match(Some(light), dark), ColorScheme::Light);
        assert_eq!(scheme_for_best_match(None, dark), ColorScheme::Light);
    }
}
