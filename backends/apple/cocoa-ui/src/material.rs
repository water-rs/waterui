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
use objc2_app_kit::{
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
};
#[cfg(target_os = "ios")]
use objc2_ui_kit::{UIBlurEffect, UIBlurEffectStyle, UIVisualEffectView};

use crate::{MainThreadMarker, PlatformView, Retained};

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

#[cfg(target_os = "macos")]
const fn material(level: MaterialLevel) -> NSVisualEffectMaterial {
    match level {
        MaterialLevel::UltraThin => NSVisualEffectMaterial::HUDWindow,
        MaterialLevel::Thin => NSVisualEffectMaterial::Titlebar,
        MaterialLevel::Regular | MaterialLevel::Thick => NSVisualEffectMaterial::Menu,
        MaterialLevel::UltraThick => NSVisualEffectMaterial::Sidebar,
    }
}

#[cfg(target_os = "macos")]
const fn blending_mode(level: MaterialLevel) -> NSVisualEffectBlendingMode {
    match level {
        MaterialLevel::UltraThin | MaterialLevel::Thin => NSVisualEffectBlendingMode::BehindWindow,
        _ => NSVisualEffectBlendingMode::WithinWindow,
    }
}

/// Creates a material background view.
///
/// On iOS the returned `UIView` is a `UIVisualEffectView`; add embedded
/// content to its `contentView()`. On macOS the returned `NSView` is an
/// `NSVisualEffectView`; add embedded content as a subview.
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
        let view = NSVisualEffectView::new(mtm);
        view.setMaterial(material(level));
        view.setBlendingMode(blending_mode(level));
        view.setState(NSVisualEffectState::Active);
        Retained::into_super(view)
    }
}
