//! A resolved color as plain components.
//!
//! Platform color types stay inside the kit's platform modules; this is the
//! value they resolve to when a specific appearance is applied.

/// A red/green/blue/alpha color, each component in `0.0` to `1.0`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rgba {
    /// The red component.
    pub red: f64,
    /// The green component.
    pub green: f64,
    /// The blue component.
    pub blue: f64,
    /// The opacity component.
    pub alpha: f64,
}

impl Rgba {
    /// An sRGB color.
    #[must_use]
    pub const fn new(red: f64, green: f64, blue: f64, alpha: f64) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    /// The same color with `alpha` replacing its opacity.
    #[must_use]
    pub const fn with_alpha(self, alpha: f64) -> Self {
        Self { alpha, ..self }
    }
}

/// Builds a four-component RGB `CGColor` in a known RGB space.
fn cg_rgb(
    space: Option<&objc2_core_graphics::CGColorSpace>,
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
) -> objc2_core_foundation::CFRetained<objc2_core_graphics::CGColor> {
    use objc2_core_graphics::CGColor;
    let components = [red, green, blue, alpha.clamp(0.0, 1.0)];
    // SAFETY: callers pass one of the typed RGB spaces below, each of which
    // takes exactly four components.
    unsafe { CGColor::new(space, components.as_ptr()) }
        .expect("the typed RGB color space and four components always make a color")
}

/// A `CGColor` in the extended linear sRGB space. Channels pass through straight:
/// values above `1.0` are the HDR headroom already encoded in the color.
///
/// # Panics
///
/// Never in practice: extended sRGB and four components always make a
/// color; the `expect` only covers a platform that does not.
#[must_use]
pub fn cg_extended_linear_srgb(
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
) -> objc2_core_foundation::CFRetained<objc2_core_graphics::CGColor> {
    use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedLinearSRGB};
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedLinearSRGB }));
    cg_rgb(space.as_deref(), red, green, blue, alpha)
}

/// A `CGColor` in the extended linear Display-P3 space. Channels pass
/// through straight, including values above `1.0`.
#[must_use]
pub fn cg_extended_linear_display_p3(
    red: f64,
    green: f64,
    blue: f64,
    alpha: f64,
) -> objc2_core_foundation::CFRetained<objc2_core_graphics::CGColor> {
    use objc2_core_graphics::{CGColorSpace, kCGColorSpaceExtendedLinearDisplayP3};
    // SAFETY: the static is a `CFString` constant exported by Core Graphics.
    let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceExtendedLinearDisplayP3 }));
    cg_rgb(space.as_deref(), red, green, blue, alpha)
}
