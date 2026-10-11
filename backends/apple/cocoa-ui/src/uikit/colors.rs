//! The platform's semantic colors, resolved to concrete components.
//!
//! `UIColor`'s semantic colors are dynamic: what they draw depends on the
//! trait collection in effect at resolution time. [`resolve`] answers under
//! a `UITraitCollection` for one interface style, so the result is for that
//! scheme exactly.
//!
//! # Safety
//!
//! The `unsafe` here reads `UIColor`'s component accessors on a color resolved
//! under a trait collection — documented API, on the calling thread.

use objc2::rc::Retained;
use objc2_core_graphics::{
    CGColor, CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3, kCGColorSpaceExtendedLinearSRGB,
    kCGColorSpaceLinearSRGB,
};
use objc2_foundation::NSString;
use objc2_ui_kit::{UIColor, UITraitCollection, UIUserInterfaceStyle};

use crate::color::Rgba;
use crate::color_scheme::ColorScheme;

/// One of `UIKit`'s semantic colors, or the app's accent color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiColor {
    /// `UIColor.systemBackgroundColor`.
    SystemBackground,
    /// `UIColor.secondarySystemBackgroundColor`.
    SecondarySystemBackground,
    /// `UIColor.tertiarySystemFillColor`.
    TertiarySystemFill,
    /// `UIColor.separatorColor`.
    Separator,
    /// `UIColor.labelColor`.
    Label,
    /// `UIColor.secondaryLabelColor`.
    SecondaryLabel,
    /// The asset catalog's `AccentColor`, or `UIColor.systemBlueColor` when
    /// the app does not define one — the same answer `UIColor.tintColor`
    /// resolves to for a stock app.
    Accent,
    /// `UIColor.whiteColor`.
    White,
    /// `UIColor.systemPurple`.
    SystemPurple,
    /// `UIColor.systemRed`.
    SystemRed,
}

fn native(color: UiColor) -> Retained<UIColor> {
    match color {
        UiColor::SystemBackground => UIColor::systemBackgroundColor(),
        UiColor::SecondarySystemBackground => UIColor::secondarySystemBackgroundColor(),
        UiColor::TertiarySystemFill => UIColor::tertiarySystemFillColor(),
        UiColor::Separator => UIColor::separatorColor(),
        UiColor::Label => UIColor::labelColor(),
        UiColor::SecondaryLabel => UIColor::secondaryLabelColor(),
        UiColor::Accent => UIColor::colorNamed(&NSString::from_str("AccentColor"))
            .unwrap_or_else(UIColor::systemBlueColor),
        UiColor::White => UIColor::whiteColor(),
        UiColor::SystemPurple => UIColor::systemPurpleColor(),
        UiColor::SystemRed => UIColor::systemRedColor(),
    }
}

/// A color in the extended linear sRGB space: `red`, `green` and `blue`
/// pass through straight — values above `1.0` are the HDR headroom already
/// encoded in the working color. `alpha` clamps to `0.0…1.0`.
///
/// # Panics
///
/// Never in practice: extended sRGB and four components always make a color;
/// the `expect` only covers a platform that does not.
#[must_use]
pub fn extended_linear_srgb(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<UIColor> {
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
) -> Retained<UIColor> {
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
) -> Retained<UIColor> {
    let components = [red, green, blue, alpha.clamp(0.0, 1.0)];
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let space = CGColorSpace::with_name(Some(space_name));
    // SAFETY: `components` points at four f64s, the count of either typed RGB
    // space above.
    let cg = unsafe { CGColor::new(space.as_deref(), components.as_ptr()) }
        .expect("the typed RGB color space and four components always make a color");
    UIColor::colorWithCGColor(&cg)
}

/// The dynamic color placeholder text draws in — the same color
/// `UITextField` uses for its placeholder, tracking the resolved interface
/// style.
#[must_use]
pub fn placeholder_text() -> Retained<UIColor> {
    UIColor::placeholderTextColor()
}

/// A color in the linear sRGB space: `red`, `green` and `blue` are sRGB
/// components, clamped to `0.0…1.0` — the SDR-only counterpart of
/// [`extended_linear_srgb`].
///
/// # Panics
///
/// Never in practice: linear sRGB and four components always make a color;
/// the `expect` only covers a platform that does not.
#[must_use]
pub fn linear(red: f64, green: f64, blue: f64, alpha: f64) -> Retained<UIColor> {
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
    UIColor::colorWithCGColor(&cg)
}

/// `color` as a `CGColor`, for installing on a `CALayer`.
#[must_use]
pub fn cg(color: &UIColor) -> Retained<CGColor> {
    // SAFETY: `CGColor` is a documented accessor on a live color; the
    // returned object is retained by the call.
    unsafe { color.CGColor() }
}

/// The same color with `alpha` replacing its opacity.
#[must_use]
pub fn with_alpha(color: &UIColor, alpha: f64) -> Retained<UIColor> {
    color.colorWithAlphaComponent(alpha)
}

/// What `color` draws as under `scheme`.
#[must_use]
pub fn resolve(color: UiColor, scheme: ColorScheme) -> Rgba {
    let style = match scheme {
        ColorScheme::Light => UIUserInterfaceStyle::Light,
        ColorScheme::Dark => UIUserInterfaceStyle::Dark,
    };
    let traits = UITraitCollection::traitCollectionWithUserInterfaceStyle(style);
    let resolved = native(color).resolvedColorWithTraitCollection(&traits);
    // SAFETY: the component accessors are valid on any `UIColor`, and the out
    // pointers point at locals that outlive the call.
    unsafe {
        let mut red = 0.0;
        let mut green = 0.0;
        let mut blue = 0.0;
        let mut alpha = 0.0;
        resolved.getRed_green_blue_alpha(
            &raw mut red,
            &raw mut green,
            &raw mut blue,
            &raw mut alpha,
        );
        Rgba::new(red, green, blue, alpha)
    }
}
