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
    NSAppearance, NSAutoresizingMaskOptions, NSBox, NSBoxType, NSColor, NSTitlePosition,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWorkspace, NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification,
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
/// `Menu`'s flat colour is darker than `thickMaterial`'s, which
/// [`flat_thick_view`] covers.
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

/// `SwiftUI`'s `thickMaterial` flat colour under Reduce Transparency, light
/// appearance: a grey of this sRGB value (240/255).
///
/// These two values reproduce `SwiftUI`'s `thickMaterial` under Reduce
/// Transparency. They are measured from its output; no published source
/// states them, and no public `NSVisualEffectMaterial` paints this light grey
/// in that mode (`Menu` paints 0.9255).
#[cfg(target_os = "macos")]
const THICK_REDUCED_TRANSPARENCY_LIGHT: f64 = 0.9412;

/// `SwiftUI`'s `thickMaterial` flat colour under Reduce Transparency, dark
/// appearance (see [`THICK_REDUCED_TRANSPARENCY_LIGHT`]); `Menu` paints the
/// same grey there.
#[cfg(target_os = "macos")]
const THICK_REDUCED_TRANSPARENCY_DARK: f64 = 0.1961;

/// Whether the system's Reduce Transparency setting is on.
#[cfg(target_os = "macos")]
#[must_use]
pub fn system_reduce_transparency() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency()
}

/// A dynamic colour that resolves to the thick material's Reduce
/// Transparency grey for the appearance it is drawn in.
#[cfg(target_os = "macos")]
fn thick_reduced_transparency_color() -> Retained<NSColor> {
    let grey = |value: f64| NSColor::colorWithSRGBRed_green_blue_alpha(value, value, value, 1.0);
    let light = grey(THICK_REDUCED_TRANSPARENCY_LIGHT);
    let dark = grey(THICK_REDUCED_TRANSPARENCY_DARK);
    let provider = block2::RcBlock::new(move |appearance: core::ptr::NonNull<NSAppearance>| {
        // SAFETY: `AppKit` passes the appearance being resolved, live for
        // the call.
        let appearance = unsafe { appearance.as_ref() };
        let chosen = match crate::appkit::color_scheme_of(appearance) {
            crate::ColorScheme::Light => &light,
            crate::ColorScheme::Dark => &dark,
        };
        // The provider's result is autoreleased, as an Objective-C method's
        // would be.
        core::ptr::NonNull::new(Retained::autorelease_ptr(chosen.clone()))
            .expect("a retained colour is non-null")
    });
    // SAFETY: the provider returns a valid, autoreleased colour.
    unsafe { NSColor::colorWithName_dynamicProvider(None, &provider) }
}

/// The flat fill of [`THICK_REDUCED_TRANSPARENCY_LIGHT`] and
/// [`THICK_REDUCED_TRANSPARENCY_DARK`] a thick material shows instead of
/// `NSVisualEffectMaterial::Menu`'s flat colour while Reduce Transparency is
/// on: a borderless `NSBox` that [`material_view_with`] adds to the effect
/// view first, so the content mounted into it later lies above and stays
/// vibrancy-hosted.
#[cfg(target_os = "macos")]
fn flat_thick_view(mtm: MainThreadMarker, visible: bool) -> Retained<NSBox> {
    let fill = NSBox::new(mtm);
    fill.setBoxType(NSBoxType::Custom);
    fill.setTitlePosition(NSTitlePosition::NoTitle);
    fill.setBorderWidth(0.0);
    fill.setFillColor(&thick_reduced_transparency_color());
    fill.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    fill.setHidden(!visible);
    fill
}

/// A material background view: the platform's effect view, plus on macOS the
/// Reduce Transparency fill a thick material carries.
#[derive(Debug, Clone)]
pub struct MaterialView {
    /// The effect view.
    view: Retained<PlatformView>,
    /// The flat fill inside the effect view; only a thick material has one.
    #[cfg(target_os = "macos")]
    fill: Option<Retained<NSBox>>,
}

impl MaterialView {
    /// The effect view; add embedded content to it as described at
    /// [`material_view`].
    #[must_use]
    pub fn view(&self) -> &PlatformView {
        &self.view
    }

    /// Shows the Reduce Transparency fill exactly when `reduce_transparency`
    /// is on. Does nothing for a material that has none.
    #[cfg(target_os = "macos")]
    pub fn set_reduce_transparency(&self, reduce_transparency: bool) {
        if let Some(fill) = &self.fill {
            fill.setHidden(!reduce_transparency);
        }
    }

    /// Follows the system's Reduce Transparency setting from now on, until
    /// the returned guard is dropped.
    #[cfg(target_os = "macos")]
    pub fn follow_system_reduce_transparency(
        &self,
        mtm: MainThreadMarker,
    ) -> crate::notification::NotificationObserver {
        // SAFETY: reads a string constant `AppKit` owns for the process's
        // lifetime.
        let name = crate::notification::NotificationName::framework(unsafe {
            NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification
        });
        let material = self.clone();
        crate::notification::observe_workspace(mtm, &name, move || {
            material.set_reduce_transparency(system_reduce_transparency());
        })
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
/// On iOS the view is a `UIVisualEffectView`; add embedded content to its
/// `contentView()`. On macOS it is a flipped `NSVisualEffectView`; add
/// embedded content as a subview, which lies above the Reduce Transparency
/// fill. On macOS a thick material follows the system's Reduce Transparency
/// setting as of this call; see [`material_view_with`] and
/// [`MaterialView::follow_system_reduce_transparency`].
#[must_use]
pub fn material_view(mtm: MainThreadMarker, level: MaterialLevel) -> MaterialView {
    #[cfg(target_os = "ios")]
    {
        let effect = UIBlurEffect::effectWithStyle(blur_style(level), mtm);
        let view = UIVisualEffectView::new(mtm);
        view.setEffect(Some(&effect));
        MaterialView {
            view: Retained::into_super(view),
        }
    }
    #[cfg(target_os = "macos")]
    {
        material_view_with(mtm, level, system_reduce_transparency())
    }
}

/// Creates a macOS material background view for a given Reduce Transparency
/// setting.
///
/// While `reduce_transparency` is on, a thick material shows `SwiftUI`'s
/// flat colour for it instead of the effect view's own; other levels are
/// unchanged. The setting is an input so it can be driven without changing
/// the system's.
#[cfg(target_os = "macos")]
#[must_use]
pub fn material_view_with(
    mtm: MainThreadMarker,
    level: MaterialLevel,
    reduce_transparency: bool,
) -> MaterialView {
    let this = EffectView::alloc(mtm);
    // SAFETY: `init` is `NSVisualEffectView`'s designated initializer.
    let view: Retained<EffectView> = unsafe { msg_send![super(this.set_ivars(())), init] };
    view.setMaterial(material(level));
    view.setBlendingMode(blending_mode(level));
    view.setState(NSVisualEffectState::Active);
    let fill = (level == MaterialLevel::Thick).then(|| flat_thick_view(mtm, reduce_transparency));
    if let Some(fill) = &fill {
        view.addSubview(fill);
    }
    MaterialView {
        view: Retained::into_super(Retained::into_super(view)),
        fill,
    }
}
