//! The `AppKit` application object of a Mac Catalyst process that runs
//! without `UIApplicationMain`.
//!
//! A Catalyst process exec'd directly has no `UIApplication`, no scene and
//! no `NSApplication`. `UIKit` still needs the last one the moment a view
//! hierarchy gets a window: a scene-less `UIWindow` attaches to the
//! screen's unassociated window scene, and that scene reads the
//! application's `AppKit` delegate, which raises an Objective-C exception
//! when no `NSApplication` exists yet. [`BackgroundApplication`] creates
//! it and holds it at the `Prohibited` activation policy, so the process
//! registers as background-only: no Dock tile, no menu bar, never active.
//!
//! The scene-less window is what gives a hierarchy the trait environment
//! and the layout pass native controls build their parts in — switch
//! knobs, slider thumbs, button bezels and steppers stay unbuilt in a
//! view that has no window — while `UIKit` creates no `NSWindow` for it.
//!
//! # Safety
//!
//! `NSApplication` has no binding on this target: the iOS SDK headers
//! `objc2-ui-kit` is generated from do not declare it. The class is looked
//! up by name in the process's own `AppKit`, which every Catalyst process
//! loads, and messaged with the signatures `AppKit` declares for it —
//! `+sharedApplication`, `-activationPolicy`, `-setActivationPolicy:` and
//! `-isActive` — on the main thread `AppKit` requires.

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{MainThreadMarker, msg_send};

/// `NSApplicationActivationPolicyProhibited`.
const ACTIVATION_POLICY_PROHIBITED: isize = 2;

/// The process's `NSApplication`, held at the `Prohibited` activation
/// policy — the proof a scene-less `UIWindow` needs before it may be
/// created (see the module documentation).
#[derive(Debug)]
pub struct BackgroundApplication {
    application: Retained<AnyObject>,
}

impl BackgroundApplication {
    /// Creates the shared `NSApplication` (or reuses the one that exists)
    /// and holds it at the `Prohibited` activation policy.
    ///
    /// An unbundled process arrives `Prohibited` already, and `AppKit`
    /// refuses the redundant change — so the policy is set only when it
    /// is not already there.
    ///
    /// # Panics
    ///
    /// When the process has no `AppKit` application class, or when
    /// `AppKit` refuses the `Prohibited` policy: a process that could
    /// take focus or show a Dock tile breaks the contract this type
    /// exists for, so a refusal is fatal rather than a fallback.
    #[must_use]
    pub fn bring_up(_mtm: MainThreadMarker) -> Self {
        let class = AnyClass::get(c"NSApplication")
            .expect("a Mac Catalyst process loads AppKit's NSApplication");
        // SAFETY: `+[NSApplication sharedApplication]` returns the process's
        // application object, creating it on first use; the marker proves
        // the main thread `AppKit` requires.
        let application: Retained<AnyObject> = unsafe { msg_send![class, sharedApplication] };
        let this = Self { application };
        if !this.is_activation_prohibited() {
            // SAFETY: `-setActivationPolicy:` takes an
            // `NSApplicationActivationPolicy` (`NSInteger`) and answers
            // `BOOL`.
            let accepted: bool = unsafe {
                msg_send![
                    &*this.application,
                    setActivationPolicy: ACTIVATION_POLICY_PROHIBITED
                ]
            };
            assert!(
                accepted,
                "AppKit refused the Prohibited activation policy for a background Catalyst process"
            );
        }
        this
    }

    /// Whether the application holds the `Prohibited` activation policy.
    #[must_use]
    pub fn is_activation_prohibited(&self) -> bool {
        // SAFETY: `-activationPolicy` answers an
        // `NSApplicationActivationPolicy` (`NSInteger`).
        let policy: isize = unsafe { msg_send![&*self.application, activationPolicy] };
        policy == ACTIVATION_POLICY_PROHIBITED
    }

    /// Whether the application is the active (focused) application.
    #[must_use]
    pub fn is_active(&self) -> bool {
        // SAFETY: `-isActive` answers `BOOL`.
        unsafe { msg_send![&*self.application, isActive] }
    }
}

/// Whether `UIKit` runs under the iPad idiom — `UIDeviceFamily` 2 in the
/// main bundle's `Info.plist`, under which Catalyst draws the iOS look
/// rather than the Mac idiom's `AppKit`-styled controls.
#[must_use]
pub fn runs_under_pad_idiom(mtm: MainThreadMarker) -> bool {
    objc2_ui_kit::UIDevice::currentDevice(mtm).userInterfaceIdiom()
        == objc2_ui_kit::UIUserInterfaceIdiom::Pad
}
