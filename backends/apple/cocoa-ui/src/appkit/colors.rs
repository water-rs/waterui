//! The platform's semantic colors, resolved to concrete components.
//!
//! `NSColor`'s semantic colors are dynamic: what they draw depends on the
//! appearance in effect at resolution time. [`resolve`] applies an
//! `NSAppearance` around the read so the answer is for one scheme exactly.
//!
//! # Safety
//!
//! The `unsafe` here asks `NSAppearance` to run a block under a named
//! appearance and reads `NSColor`'s component accessors inside it. Both are
//! documented APIs; the block runs synchronously on the calling thread while
//! that appearance is in effect.

use core::cell::RefCell;

use objc2::rc::Retained;
use objc2_app_kit::{
    NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSColor, NSColorSpace,
};
use objc2_core_graphics::{
    CGColor, CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3, kCGColorSpaceExtendedLinearSRGB,
    kCGColorSpaceLinearSRGB,
};

use crate::color::Rgba;
use crate::color_scheme::ColorScheme;

/// One of `AppKit`'s semantic colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppColor {
    /// `NSColor.windowBackgroundColor`.
    WindowBackground,
    /// `NSColor.controlBackgroundColor`.
    ControlBackground,
    /// `NSColor.tertiarySystemFill`.
    TertiarySystemFill,
    /// `NSColor.separatorColor`.
    Separator,
    /// `NSColor.labelColor`.
    Label,
    /// `NSColor.secondaryLabelColor`.
    SecondaryLabel,
    /// `NSColor.controlAccentColor`.
    ControlAccent,
    /// `NSColor.alternateSelectedControlTextColor`.
    AlternateSelectedControlText,
    /// `NSColor.selectedContentBackgroundColor`.
    SelectedContentBackground,
    /// `NSColor.systemPurple`.
    SystemPurple,
    /// `NSColor.systemRed`.
    SystemRed,
    /// `NSColor.whiteColor`.
    White,
}

fn native(color: AppColor) -> Retained<NSColor> {
    match color {
        AppColor::WindowBackground => NSColor::windowBackgroundColor(),
        AppColor::ControlBackground => NSColor::controlBackgroundColor(),
        AppColor::TertiarySystemFill => NSColor::tertiarySystemFillColor(),
        AppColor::Separator => NSColor::separatorColor(),
        AppColor::Label => NSColor::labelColor(),
        AppColor::SecondaryLabel => NSColor::secondaryLabelColor(),
        AppColor::ControlAccent => NSColor::controlAccentColor(),
        AppColor::AlternateSelectedControlText => NSColor::alternateSelectedControlTextColor(),
        AppColor::SelectedContentBackground => NSColor::selectedContentBackgroundColor(),
        AppColor::SystemPurple => NSColor::systemPurpleColor(),
        AppColor::SystemRed => NSColor::systemRedColor(),
        AppColor::White => NSColor::whiteColor(),
    }
}

/// A color in the extended linear sRGB space. Channels pass through straight,
/// including values above `1.0` for HDR content; alpha clamps to `0.0…1.0`.
///
/// # Panics
///
/// Never in practice: extended sRGB and four components always make a color,
/// and the `expect`s only cover a platform that does not.
#[must_use]
pub fn extended_linear_srgb(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<NSColor> {
    extended_linear(
        // SAFETY: the static is a `CFString` constant exported by Core Graphics.
        unsafe { kCGColorSpaceExtendedLinearSRGB },
        red,
        green,
        blue,
        alpha,
    )
}

/// A color in extended linear Display-P3, with HDR carried directly in the
/// RGB channels and straight alpha.
#[must_use]
pub fn extended_linear_display_p3(
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
) -> Retained<NSColor> {
    extended_linear(
        // SAFETY: the static is a `CFString` constant exported by Core Graphics.
        unsafe { kCGColorSpaceExtendedLinearDisplayP3 },
        red,
        green,
        blue,
        alpha,
    )
}

fn extended_linear(
    space_name: &'static objc2_core_foundation::CFString,
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
) -> Retained<NSColor> {
    let components = [red, green, blue, alpha.clamp(0.0, 1.0)];
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let space = CGColorSpace::with_name(Some(space_name));
    // SAFETY: `components` points at four f64s, the count of either typed RGB
    // space above.
    let cg = unsafe { CGColor::new(space.as_deref(), components.as_ptr()) }
        .expect("the typed RGB color space and four components always make a color");
    NSColor::colorWithCGColor(&cg).expect("every typed RGB CGColor becomes an NSColor")
}

/// The dynamic color placeholder text draws in — the same color
/// `NSTextField` uses for its placeholder, tracking the effective
/// appearance.
#[must_use]
pub fn placeholder_text() -> Retained<NSColor> {
    NSColor::placeholderTextColor()
}

/// A color in the linear sRGB space: `red`, `green` and `blue` are sRGB
/// components, clamped to `0.0…1.0` — the SDR-only counterpart of
/// [`extended_linear_srgb`].
///
/// # Panics
///
/// Never in practice: linear sRGB and four components always make a color,
/// and the `expect`s only cover a platform that does not.
#[must_use]
pub fn linear(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<NSColor> {
    let components = [
        red.clamp(0.0, 1.0),
        green.clamp(0.0, 1.0),
        blue.clamp(0.0, 1.0),
        alpha.clamp(0.0, 1.0),
    ];
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceLinearSRGB }));
    // SAFETY: `components` points at four f64s, the count sRGB takes.
    let cg = unsafe { CGColor::new(space.as_deref(), components.as_ptr()) }
        .expect("linear sRGB and four components always make a color");
    NSColor::colorWithCGColor(&cg).expect("every linear-sRGB CGColor becomes an NSColor")
}

/// `color` split into the SDR base it was exposed from and the HDR headroom
/// it carries — `linearExposure − 1`, `0` for ordinary colors.
#[must_use]
pub fn sdr_base_and_headroom(color: &NSColor) -> (Retained<NSColor>, f64) {
    let exposure = color.linearExposure();
    if exposure > 1.0 {
        (color.standardDynamicRangeColor(), exposure - 1.0)
    } else {
        (color.into(), 0.0)
    }
}

/// `color` as a `CGColor`, for installing on a `CALayer`.
#[must_use]
pub fn cg(color: &NSColor) -> Retained<CGColor> {
    color.CGColor()
}

/// The same color with `alpha` replacing its opacity.
#[must_use]
pub fn with_alpha(color: &NSColor, alpha: f64) -> Retained<NSColor> {
    color.colorWithAlphaComponent(alpha)
}

/// `color` converted into the extended sRGB — or plain sRGB — space and
/// read as components, using the appearance in effect at this moment.
///
/// `None` only for a color with no sRGB form at all, like a pattern image.
#[must_use]
pub fn srgb_components(color: &NSColor) -> Option<Rgba> {
    rgba_in(color, &NSColorSpace::extendedSRGBColorSpace())
        .or_else(|| rgba_in(color, &NSColorSpace::sRGBColorSpace()))
}

/// `color` converted into `space` and read as components.
fn rgba_in(color: &NSColor, space: &NSColorSpace) -> Option<Rgba> {
    let converted = color.colorUsingColorSpace(space)?;
    // SAFETY: the component accessors are valid on a converted color, and
    // the out pointers point at locals that outlive the call.
    unsafe {
        let mut red = 0.0;
        let mut green = 0.0;
        let mut blue = 0.0;
        let mut alpha = 0.0;
        converted.getRed_green_blue_alpha(
            &raw mut red,
            &raw mut green,
            &raw mut blue,
            &raw mut alpha,
        );
        Some(Rgba::new(red, green, blue, alpha))
    }
}

/// `color` reinterpreted as HDR content: components keep their values while
/// `headroom` (a linear exposure multiplier, `1.0` and up) declares how far
/// beyond SDR white they reach.
///
/// Falls back to `color` itself when its space has no extended-range form.
#[must_use]
pub fn with_content_headroom(color: &NSColor, headroom: f64) -> Retained<NSColor> {
    color.colorByApplyingContentHeadroom(headroom)
}

/// What `color` draws as under `scheme`.
///
/// The color is resolved under a named appearance rather than the
/// application's current one, so the answer is exact for either scheme even
/// while the user sees the other.
///
/// A dynamic color that declines to resolve in sRGB — a pattern image, for
/// example — answers `None`; semantic colors always resolve.
///
/// # Panics
///
/// Never in practice: `AppKit` ships appearances under both standard names,
/// and the `expect` only covers a platform that does not.
#[must_use]
pub fn resolve(color: AppColor, scheme: ColorScheme) -> Option<Rgba> {
    // SAFETY: these are the platform's two standard appearance names.
    let name = unsafe {
        match scheme {
            ColorScheme::Light => NSAppearanceNameAqua,
            ColorScheme::Dark => NSAppearanceNameDarkAqua,
        }
    };
    let appearance = NSAppearance::appearanceNamed(name)
        .expect("AppKit ships appearances under both standard names");
    let color = native(color);
    let rgba = RefCell::new(None);
    {
        let slot = &rgba;
        let block = block2::RcBlock::new(move || {
            *slot.borrow_mut() = rgba_of(&color);
        });
        appearance.performAsCurrentDrawingAppearance(&block);
    }
    rgba.into_inner()
}

/// `color` converted into the sRGB space and read as components, using the
/// appearance in effect at this moment.
fn rgba_of(color: &NSColor) -> Option<Rgba> {
    let converted = color.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
    // SAFETY: the component accessors are valid on a color converted to sRGB,
    // and the out pointers point at locals that outlive the call.
    unsafe {
        let mut red = 0.0;
        let mut green = 0.0;
        let mut blue = 0.0;
        let mut alpha = 0.0;
        converted.getRed_green_blue_alpha(
            &raw mut red,
            &raw mut green,
            &raw mut blue,
            &raw mut alpha,
        );
        Some(Rgba::new(red, green, blue, alpha))
    }
}
