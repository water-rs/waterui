//! Conversions between the authoring spaces and Cherenkov's working colour.
//!
//! [`WorkingColor`] is linear Display P3 with straight alpha and an extended
//! range: values above `1.0` are brighter than SDR white. The authoring
//! spaces (`Srgb`, `P3`, `Oklch`) resolve into it, and the perceptual
//! adjustments on `Color` go through Oklch and back.

use cherenkov::{Color as SpaceColor, LinearSrgb, WorkingColor};
use color::{AlphaColor, Oklch as OklchSpace};

use super::{Oklch, Srgb, linear_to_srgb};

/// Builds a working colour from linear sRGB components and an alpha.
#[must_use]
pub fn from_linear_srgb([red, green, blue]: [f32; 3], alpha: f32) -> WorkingColor {
    SpaceColor::<LinearSrgb>::new([red, green, blue, alpha]).into()
}

/// The linear sRGB components of a working colour, alpha dropped.
#[must_use]
pub fn to_linear_srgb(color: WorkingColor) -> [f32; 3] {
    let [red, green, blue, _] = color.components;
    let converted = AlphaColor::<cherenkov::LinearDisplayP3>::new([red, green, blue, 1.0])
        .convert::<LinearSrgb>()
        .components;
    [converted[0], converted[1], converted[2]]
}

/// The gamma-encoded sRGB colour nearest to a working colour, alpha dropped.
#[must_use]
pub fn to_srgb(color: WorkingColor) -> Srgb {
    let [red, green, blue] = to_linear_srgb(color);
    Srgb::new(
        linear_to_srgb(red),
        linear_to_srgb(green),
        linear_to_srgb(blue),
    )
}

/// The Oklch coordinates of a working colour, alpha dropped.
#[must_use]
pub fn to_oklch(color: WorkingColor) -> Oklch {
    let [red, green, blue, _] = color.components;
    let [lightness, chroma, hue, _] =
        AlphaColor::<cherenkov::LinearDisplayP3>::new([red, green, blue, 1.0])
            .convert::<OklchSpace>()
            .components;
    Oklch::new(lightness, chroma, if hue.is_finite() { hue } else { 0.0 })
}

/// Builds a working colour from Oklch coordinates and an alpha.
#[must_use]
pub fn from_oklch(oklch: Oklch, alpha: f32) -> WorkingColor {
    let converted =
        AlphaColor::<OklchSpace>::new([oklch.lightness, oklch.chroma, oklch.hue, alpha])
            .convert::<cherenkov::LinearDisplayP3>()
            .components;
    WorkingColor::new(converted)
}

/// Scales the colour channels by `1 + headroom`, brightening it past SDR white.
///
/// Non-finite or negative headroom is no headroom.
#[must_use]
pub fn with_headroom(color: WorkingColor, headroom: f32) -> WorkingColor {
    let headroom = if headroom.is_finite() && headroom > 0.0 {
        headroom
    } else {
        0.0
    };
    let scale = 1.0 + headroom;
    let [red, green, blue, alpha] = color.components;
    WorkingColor::new([red * scale, green * scale, blue * scale, alpha])
}

/// Linearly interpolates every component, `t` clamped to `[0, 1]`.
#[must_use]
pub fn lerp(first: WorkingColor, second: WorkingColor, t: f32) -> WorkingColor {
    let t = t.clamp(0.0, 1.0);
    let mut components = [0.0; 4];
    for (index, out) in components.iter_mut().enumerate() {
        *out = (second.components[index] - first.components[index])
            .mul_add(t, first.components[index]);
    }
    WorkingColor::new(components)
}
