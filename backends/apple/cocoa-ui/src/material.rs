//! Material background surfaces.
//!
//! iOS uses a `UIVisualEffectView` driven by a `UIBlurEffect`; macOS uses an
//! `NSVisualEffectView` with a material, blending mode, and active state.
//! The creation API is unified so callers can host content in the same way
//! on both platforms.
//!
//! # Safety
//!
//! No unsafe APIs are used here beyond the platform view initializers that
//! require a [`MainThreadMarker`]; the returned views are ordinary platform
//! views.

#[cfg(target_os = "macos")]
use objc2::{MainThreadOnly, define_class, msg_send, runtime::NSObjectProtocol};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
};
#[cfg(target_os = "ios")]
use objc2_ui_kit::{UIBlurEffect, UIBlurEffectStyle, UIVisualEffectView};

use crate::{MainThreadMarker, PlatformView, Retained};

#[cfg(target_os = "macos")]
define_class!(
    // SAFETY: `NSVisualEffectView` asks a subclass to initialize through its
    // designated initializer, which `material_view` does, and the class does
    // not implement `Drop`.
    #[unsafe(super(NSVisualEffectView))]
    #[name = "CocoaUiEffectView"]
    #[thread_kind = MainThreadOnly]
    #[derive(Debug)]
    /// The `NSVisualEffectView` a material is realized as.
    ///
    /// Its coordinates are flipped, like a `HostView`'s, so content laid out
    /// in a host's coordinates keeps its frame when it is mounted inside the
    /// effect view.
    struct EffectView;

    // SAFETY: `NSObjectProtocol` asks nothing of an `NSView` subclass.
    unsafe impl NSObjectProtocol for EffectView {}

    impl EffectView {
        // SAFETY: `isFlipped` is `NSView`'s documented override point.
        #[unsafe(method(isFlipped))]
        fn is_flipped_override(&self) -> bool {
            true
        }
    }
);

/// The thickness of a material background, ordered from most transparent to
/// most opaque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialLevel {
    /// The thinnest material.
    UltraThin,
    /// A thin material.
    Thin,
    /// The standard material.
    Regular,
    /// A thick material.
    Thick,
    /// The thickest material.
    UltraThick,
}

#[cfg(target_os = "ios")]
const fn blur_style(level: MaterialLevel) -> UIBlurEffectStyle {
    match level {
        MaterialLevel::UltraThin => UIBlurEffectStyle::SystemUltraThinMaterial,
        MaterialLevel::Thin => UIBlurEffectStyle::SystemThinMaterial,
        MaterialLevel::Regular => UIBlurEffectStyle::SystemMaterial,
        MaterialLevel::Thick => UIBlurEffectStyle::SystemThickMaterial,
        MaterialLevel::UltraThick => UIBlurEffectStyle::SystemChromeMaterial,
    }
}

/// The within-window levels realize the `AppKit` material that carries
/// `SwiftUI`'s recipe for the same thickness (backdrop tint and opacity,
/// clamp colour and saturation, in both appearances): `Popover` is
/// `regularMaterial`, `Menu` is `thickMaterial` and `Sidebar` is
/// `ultraThickMaterial`. Under Reduce Transparency in the light appearance,
/// `Menu`'s flat colour is darker than `thickMaterial`'s.
#[cfg(target_os = "macos")]
const fn material(level: MaterialLevel) -> NSVisualEffectMaterial {
    match level {
        MaterialLevel::UltraThin => NSVisualEffectMaterial::HUDWindow,
        MaterialLevel::Thin => NSVisualEffectMaterial::Titlebar,
        MaterialLevel::Regular => NSVisualEffectMaterial::Popover,
        MaterialLevel::Thick => NSVisualEffectMaterial::Menu,
        MaterialLevel::UltraThick => NSVisualEffectMaterial::Sidebar,
    }
}

/// Whether `level` blends what lies behind the window — the desktop —
/// rather than the window's own content. A window whose background is such a
/// level must be non-opaque for the desktop to show through.
#[cfg(target_os = "macos")]
#[must_use]
pub const fn blends_behind_window(level: MaterialLevel) -> bool {
    matches!(level, MaterialLevel::UltraThin | MaterialLevel::Thin)
}

#[cfg(target_os = "macos")]
const fn blending_mode(level: MaterialLevel) -> NSVisualEffectBlendingMode {
    if blends_behind_window(level) {
        NSVisualEffectBlendingMode::BehindWindow
    } else {
        NSVisualEffectBlendingMode::WithinWindow
    }
}

/// Creates a material background view.
///
/// On iOS the returned `UIView` is a `UIVisualEffectView`; add embedded
/// content to its `contentView()`. On macOS the returned `NSView` is an
/// flipped `NSVisualEffectView`; add embedded content as a subview.
#[must_use]
pub fn material_view(mtm: MainThreadMarker, level: MaterialLevel) -> Retained<PlatformView> {
    #[cfg(target_os = "ios")]
    {
        let effect = UIBlurEffect::effectWithStyle(blur_style(level), mtm);
        let view = UIVisualEffectView::new(mtm);
        view.setEffect(Some(&effect));
        Retained::into_super(view)
    }
    #[cfg(target_os = "macos")]
    {
        let this = EffectView::alloc(mtm);
        // SAFETY: `init` is `NSVisualEffectView`'s designated initializer.
        let view: Retained<EffectView> = unsafe { msg_send![super(this.set_ivars(())), init] };
        view.setMaterial(material(level));
        view.setBlendingMode(blending_mode(level));
        view.setState(NSVisualEffectState::Active);
        Retained::into_super(Retained::into_super(view))
    }
}
