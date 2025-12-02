//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports sRGB, Display P3, and OKLCH color spaces, with utilities for
//! conversion and manipulation of color values.
//!
//! The OKLCH color space is perceptually uniform and is therefore recommended
//! for most user interface work. By expressing colors in OKLCH you can adjust
//! lightness, chroma, and hue independently while maintaining predictable
//! contrast relationships across themes.
//!
//! The primary type is `Color`, which can represent colors in sRGB, Display P3,
//! or OKLCH color spaces, with conversion methods from various tuple formats.

mod oklch;
pub use oklch::Oklch;
mod p3;
pub use p3::P3;
mod srgb;
pub use srgb::Srgb;

use core::{
    fmt::{self, Debug, Display},
    ops::{Deref, DerefMut},
};

use nami::{Computed, Signal, SignalExt, impl_constant};

use waterui_core::{
    Environment, raw_view,
    layout::StretchAxis,
    resolve::{self, AnyResolvable, Resolvable},
};

/// A color value that can be resolved in different color spaces.
///
/// This is the main color type that wraps a resolvable color value.
/// Colors can be created from sRGB, P3, OKLCH, or custom color spaces.
///
/// # Layout Behavior
///
/// Color is a **greedy view** that expands to fill all available space in both
/// directions. Use `.frame()` to constrain its size, or use it as a background.
///
/// # Examples
///
/// ```ignore
/// // Fills entire container
/// Color::blue()
///
/// // Constrained to specific size
/// Color::red().frame().width(100.0).height(50.0)
///
/// // As a background
/// text("Hello").background(Color::yellow())
/// ```
//
// ═══════════════════════════════════════════════════════════════════════════
// INTERNAL: Layout Contract for Backend Implementers
// ═══════════════════════════════════════════════════════════════════════════
//

// With constraints: Returns the full proposal size
// Without constraints: Returns a small fallback (e.g., 10pt × 10pt)
//
// ═══════════════════════════════════════════════════════════════════════════
//
#[derive(Debug, Clone)]
pub struct Color(AnyResolvable<ResolvedColor>);

impl Default for Color {
    fn default() -> Self {
        Self::srgb(0, 0, 0)
    }
}

impl_constant!(ResolvedColor);

impl<T: Resolvable<Resolved = ResolvedColor> + 'static> From<T> for Color {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

/// Represents a color with an opacity/alpha value applied.
///
/// This wrapper type allows applying a specific opacity to any color type.
#[derive(Debug, Clone)]
pub struct WithOpacity<T> {
    color: T,
    opacity: f32,
}

impl<T> WithOpacity<T> {
    /// Creates a new color with the specified opacity applied.
    ///
    /// # Arguments
    /// * `color` - The base color
    /// * `opacity` - Opacity value (0.0 = transparent, 1.0 = opaque)
    #[must_use]
    pub const fn new(color: T, opacity: f32) -> Self {
        Self { color, opacity }
    }
}

impl<T> Resolvable for WithOpacity<T>
where
    T: Resolvable<Resolved = ResolvedColor> + 'static,
{
    type Resolved = ResolvedColor;
    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let opacity = self.opacity;
        self.color.resolve(env).map(move |mut resolved| {
            resolved.opacity = opacity;
            resolved
        })
    }
}

impl<T> Deref for WithOpacity<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        &self.color
    }
}

impl<T> DerefMut for WithOpacity<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.color
    }
}

/// Errors that can occur when parsing hexadecimal color strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HexColorError {
    /// The provided string does not have the expected 6 hexadecimal digits.
    InvalidLength,
    /// A non-hexadecimal character was encountered at the provided index.
    InvalidDigit(usize),
}

impl Display for HexColorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => f.write_str("expected exactly 6 hexadecimal digits"),
            Self::InvalidDigit(index) => {
                write!(f, "invalid hexadecimal digit at byte index {index}")
            }
        }
    }
}

mod parse;

/// Represents a resolved color in linear sRGB color space with extended range support.
///
/// This struct stores color components in linear RGB values (0.0-1.0 for standard sRGB,
/// values outside this range represent colors in extended color spaces like P3).
#[derive(Debug, Clone, Copy)]
pub struct ResolvedColor {
    /// Red component in linear RGB (0.0-1.0 for sRGB, <0 or >1 for P3)
    pub red: f32,
    /// Green component in linear RGB (0.0-1.0 for sRGB, <0 or >1 for P3)
    pub green: f32,
    /// Blue component in linear RGB (0.0-1.0 for sRGB, <0 or >1 for P3)
    pub blue: f32,
    /// Extended color range headroom value (positive values allow for HDR colors)
    pub headroom: f32,
    /// Opacity/alpha channel (0.0 = transparent, 1.0 = opaque)
    pub opacity: f32,
}

impl ResolvedColor {
    /// Creates a resolved color from an sRGB color with default metadata.
    #[must_use]
    pub fn from_srgb(color: Srgb) -> Self {
        color.resolve()
    }

    /// Converts this resolved color back into sRGB space (with gamma correction).
    #[must_use]
    pub fn to_srgb(&self) -> Srgb {
        Srgb::new(
            linear_to_srgb(self.red),
            linear_to_srgb(self.green),
            linear_to_srgb(self.blue),
        )
    }

    /// Converts this resolved color into the OKLCH color space.
    #[must_use]
    pub fn to_oklch(&self) -> Oklch {
        linear_srgb_to_oklch(self.red, self.green, self.blue)
    }

    /// Creates a resolved color from an OKLCH color with the provided metadata.
    #[must_use]
    pub fn from_oklch(oklch: Oklch, headroom: f32, opacity: f32) -> Self {
        let [red, green, blue] = oklch_to_linear_srgb(oklch.lightness, oklch.chroma, oklch.hue);
        Self {
            red,
            green,
            blue,
            headroom,
            opacity,
        }
    }

    /// Returns a copy of this color with the provided opacity.
    #[must_use]
    pub const fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    /// Returns a copy of this color with the provided headroom value.
    #[must_use]
    pub const fn with_headroom(mut self, headroom: f32) -> Self {
        self.headroom = headroom;
        self
    }

    /// Linearly interpolates between this color and another color.
    #[must_use]
    pub fn lerp(self, other: Self, factor: f32) -> Self {
        let t = factor.clamp(0.0, 1.0);
        Self {
            red: lerp(self.red, other.red, t),
            green: lerp(self.green, other.green, t),
            blue: lerp(self.blue, other.blue, t),
            headroom: lerp(self.headroom, other.headroom, t),
            opacity: lerp(self.opacity, other.opacity, t),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd, Hash, Eq, Ord)]
#[non_exhaustive]
/// Represents the supported color spaces for color representation.
pub enum Colorspace {
    /// Standard RGB color space (sRGB) with values typically in the range 0-255.
    #[default]
    Srgb,
    /// Display P3 color space with extended color gamut, using floating-point values 0.0-1.0.
    P3,
    /// Perceptually uniform OKLCH color space (recommended for UI work).
    Oklch,
}

impl_constant!(Color);

impl Color {
    /// Creates a new color from a custom resolvable color value.
    ///
    /// # Arguments
    /// * `custom` - A resolvable color implementation
    pub fn new(custom: impl Resolvable<Resolved = ResolvedColor> + 'static) -> Self {
        Self(AnyResolvable::new(custom))
    }

    fn map_resolved(self, func: impl Fn(ResolvedColor) -> ResolvedColor + Clone + 'static) -> Self {
        Self::new(resolve::Map::new(self.0, func))
    }

    fn map_oklch(self, func: impl Fn(Oklch) -> Oklch + Clone + 'static) -> Self {
        self.map_resolved(move |resolved| {
            let base = resolved.to_oklch();
            let mut mapped = func(base);

            if !mapped.lightness.is_finite() {
                mapped.lightness = base.lightness;
            }
            mapped.lightness = clamp_unit(mapped.lightness);

            if !mapped.chroma.is_finite() {
                mapped.chroma = base.chroma;
            }
            mapped.chroma = clamp_non_negative(mapped.chroma);

            if !mapped.hue.is_finite() {
                mapped.hue = base.hue;
            }
            mapped.hue = normalize_hue(mapped.hue);

            ResolvedColor::from_oklch(mapped, resolved.headroom, resolved.opacity)
        })
    }

    fn adjust_lightness(self, delta: f32) -> Self {
        self.map_oklch(move |mut color| {
            color.lightness = clamp_unit(color.lightness + delta);
            color
        })
    }

    fn adjust_chroma(self, scale: f32) -> Self {
        self.map_oklch(move |mut color| {
            let factor = scale.max(0.0);
            color.chroma = clamp_non_negative(color.chroma * factor);
            color
        })
    }

    /// Creates an sRGB color from 8-bit color components.
    ///
    /// # Arguments
    /// * `red` - Red component (0-255)
    /// * `green` - Green component (0-255)
    /// * `blue` - Blue component (0-255)
    #[must_use]
    pub fn srgb(red: u8, green: u8, blue: u8) -> Self {
        Self::new(Srgb::new(
            f32::from(red) / 255.0,
            f32::from(green) / 255.0,
            f32::from(blue) / 255.0,
        ))
    }

    /// Creates an sRGB color from floating-point color components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use]
    pub fn srgb_f32(red: f32, green: f32, blue: f32) -> Self {
        Self::new(Srgb::new(red, green, blue))
    }

    /// Creates a P3 color from floating-point color components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use]
    pub fn p3(red: f32, green: f32, blue: f32) -> Self {
        Self::new(P3::new(red, green, blue))
    }

    /// Creates an OKLCH color from perceptual lightness, chroma, and hue values.
    ///
    /// OKLCH is recommended for authoring UI colors due to its perceptual
    /// uniformity.
    #[must_use]
    pub fn oklch(lightness: f32, chroma: f32, hue: f32) -> Self {
        Self::new(Oklch::new(lightness, chroma, hue))
    }

    /// Creates an sRGB color from a hexadecimal color string.
    ///
    /// Panics if the string does not contain exactly six hexadecimal digits.
    #[must_use]
    pub fn srgb_hex(hex: &str) -> Self {
        Self::new(Srgb::from_hex(hex))
    }

    /// Tries to create an sRGB color from a hexadecimal color string.
    ///
    /// # Errors
    ///
    /// Returns [`HexColorError`] if the provided string is not a valid six-digit
    /// hexadecimal color.
    pub fn try_srgb_hex(hex: &str) -> Result<Self, HexColorError> {
        Srgb::try_from_hex(hex).map(Self::from)
    }

    /// Creates an sRGB color from a packed 0xRRGGBB value.
    #[must_use]
    pub fn srgb_u32(rgb: u32) -> Self {
        Self::from(Srgb::from_u32(rgb))
    }

    /// Returns a fully transparent color.
    #[must_use]
    pub fn transparent() -> Self {
        Self::srgb(0, 0, 0).with_opacity(0.0)
    }

    /// Creates a new color with the specified opacity applied.
    ///
    /// # Arguments
    /// * `opacity` - Opacity value (0.0 = transparent, 1.0 = opaque)
    #[must_use]
    pub fn with_opacity(self, opacity: f32) -> Self {
        let clamped = clamp_unit(opacity);
        self.map_resolved(move |resolved| resolved.with_opacity(clamped))
    }

    /// Alias for [`with_opacity`].
    #[must_use]
    pub fn with_alpha(self, opacity: f32) -> Self {
        self.with_opacity(opacity)
    }

    /// Creates a new color with extended headroom for HDR content.
    ///
    /// # Arguments
    /// * `headroom` - Additional headroom value for extended range
    #[must_use]
    pub fn with_headroom(self, headroom: f32) -> Self {
        let clamped = clamp_non_negative(headroom);
        self.map_resolved(move |resolved| resolved.with_headroom(clamped))
    }

    /// Lightens the color by increasing its OKLCH lightness component.
    #[must_use]
    pub fn lighten(self, amount: f32) -> Self {
        self.adjust_lightness(clamp_unit(amount.max(0.0)))
    }

    /// Darkens the color by decreasing its OKLCH lightness component.
    #[must_use]
    pub fn darken(self, amount: f32) -> Self {
        self.adjust_lightness(-clamp_unit(amount.max(0.0)))
    }

    /// Adjusts the color saturation by scaling the OKLCH chroma component.
    #[must_use]
    pub fn saturate(self, amount: f32) -> Self {
        self.adjust_chroma(1.0 + amount)
    }

    /// Decreases the color saturation by scaling the OKLCH chroma component down.
    #[must_use]
    pub fn desaturate(self, amount: f32) -> Self {
        self.adjust_chroma(1.0 - clamp_unit(amount.max(0.0)))
    }

    /// Rotates the color's hue by the provided number of degrees.
    #[must_use]
    pub fn hue_rotate(self, degrees: f32) -> Self {
        self.map_oklch(move |mut color| {
            color.hue = normalize_hue(color.hue + degrees);
            color
        })
    }

    /// Mixes this color with another color using linear interpolation.
    #[must_use]
    pub fn mix(self, other: impl Into<Self>, factor: f32) -> Self {
        let other = other.into();
        Self::new(Mix {
            first: self.0,
            second: other.0,
            factor: clamp_unit(factor),
        })
    }

    /// Resolves this color to a concrete color value in the given environment.
    ///
    /// # Arguments
    /// * `env` - The environment to resolve the color in
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<ResolvedColor> {
        self.0.resolve(env)
    }
}

#[derive(Debug, Clone)]
struct Mix {
    first: AnyResolvable<ResolvedColor>,
    second: AnyResolvable<ResolvedColor>,
    factor: f32,
}

impl Resolvable for Mix {
    type Resolved = ResolvedColor;

    fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let factor = self.factor;
        self.first
            .resolve(env)
            .zip(self.second.resolve(env))
            .map(move |(a, b)| a.lerp(b, factor))
    }
}

macro_rules! color_const {
    ($name:ident, $color:expr,$doc:expr) => {
        #[derive(Debug, Clone, Copy)]
        #[doc=$doc]
        pub struct $name;

        impl Resolvable for $name {
            type Resolved = ResolvedColor;
            fn resolve(&self, env: &Environment) -> impl Signal<Output = Self::Resolved> {
                env.query::<Self, ResolvedColor>()
                    .copied()
                    .unwrap_or_else(|| $color.resolve())
            }
        }

        impl waterui_core::View for $name {
            fn body(self, _env: &waterui_core::Environment) -> impl waterui_core::View {
                Color::new(self)
            }
        }
    };
}

color_const!(Red, Srgb::RED, "Red color.");
color_const!(Pink, Srgb::PINK, "Pink color.");
color_const!(Purple, Srgb::PURPLE, "Purple color.");
color_const!(DeepPurple, Srgb::DEEP_PURPLE, "Deep purple color.");
color_const!(Indigo, Srgb::INDIGO, "Indigo color.");
color_const!(Blue, Srgb::BLUE, "Blue color.");
color_const!(LightBlue, Srgb::LIGHT_BLUE, "Light blue color.");
color_const!(Cyan, Srgb::CYAN, "Cyan color.");
color_const!(Teal, Srgb::TEAL, "Teal color.");
color_const!(Green, Srgb::GREEN, "Green color.");
color_const!(LightGreen, Srgb::LIGHT_GREEN, "Light green color.");
color_const!(Lime, Srgb::LIME, "Lime color.");
color_const!(Yellow, Srgb::YELLOW, "Yellow color.");
color_const!(Amber, Srgb::AMBER, "Amber color.");
color_const!(Orange, Srgb::ORANGE, "Orange color.");
color_const!(DeepOrange, Srgb::DEEP_ORANGE, "Deep orange color.");
color_const!(Brown, Srgb::BROWN, "Brown color.");

color_const!(Grey, Srgb::GREY, "Grey color.");
color_const!(BlueGrey, Srgb::BLUE_GREY, "Blue grey color.");
raw_view!(Color, StretchAxis::Both);

// https://www.w3.org/TR/css-color-4/#color-conversion-code
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055_f32.mul_add(c.powf(1.0 / 2.4), -0.055)
    }
}

// Conversion matrix from P3 to sRGB
// https://www.w3.org/TR/css-color-4/#color-conversion-code
fn p3_to_linear_srgb(p3: [f32; 3]) -> [f32; 3] {
    [
        1.224_940_1_f32.mul_add(p3[0], -0.224_940_1 * p3[1]),
        (-0.042_030_1_f32).mul_add(p3[0], 1.042_030_1 * p3[1]),
        (-0.019_721_1_f32).mul_add(
            p3[0],
            (-0.078_636_1_f32).mul_add(p3[1], 1.098_357_2 * p3[2]),
        ),
    ]
}

// Conversion matrix from sRGB to P3 (inverse of p3_to_linear_srgb)
// https://www.w3.org/TR/css-color-4/#color-conversion-code
fn linear_srgb_to_p3(srgb: [f32; 3]) -> [f32; 3] {
    [
        0.822_461_9_f32.mul_add(srgb[0], 0.177_538_1 * srgb[1]),
        0.033_194_2_f32.mul_add(srgb[0], 0.966_805_8 * srgb[1]),
        0.017_082_6_f32.mul_add(
            srgb[0],
            0.072_397_4_f32.mul_add(srgb[1], 0.910_519_9 * srgb[2]),
        ),
    ]
}

#[allow(
    clippy::excessive_precision,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]
fn linear_srgb_to_oklab(red: f32, green: f32, blue: f32) -> [f32; 3] {
    let l = 0.412_221_470_8_f32.mul_add(red, 0.536_332_536_3 * green) + 0.051_445_992_9 * blue;
    let m = 0.211_903_498_2_f32.mul_add(red, 0.680_699_545_1 * green) + 0.107_396_956_6 * blue;
    let s = 0.088_302_461_9_f32.mul_add(red, 0.281_718_837_6 * green) + 0.629_978_700_5 * blue;

    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();

    [
        0.210_454_255_3_f32.mul_add(l_, 0.793_617_785 * m_) - 0.004_072_046_8 * s_,
        1.977_998_495_1_f32.mul_add(l_, (-2.428_592_205_f32).mul_add(m_, 0.450_593_709_9 * s_)),
        0.025_904_037_1_f32.mul_add(l_, 0.782_771_766_2 * m_) - 0.808_675_766 * s_,
    ]
}

#[allow(
    clippy::excessive_precision,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]
fn linear_srgb_to_oklch(red: f32, green: f32, blue: f32) -> Oklch {
    let [lightness, a, b] = linear_srgb_to_oklab(red, green, blue);
    let chroma = a.hypot(b);
    let mut hue = b.atan2(a).to_degrees();
    if hue < 0.0 {
        hue += 360.0;
    }

    Oklch::new(lightness, chroma, hue)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    (b - a).mul_add(t, a)
}

const fn clamp_unit(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

const fn clamp_non_negative(value: f32) -> f32 {
    value.max(0.0)
}

fn normalize_hue(mut hue: f32) -> f32 {
    hue %= 360.0;
    if hue < 0.0 {
        hue += 360.0;
    }
    hue
}

#[allow(
    clippy::excessive_precision,
    clippy::many_single_char_names,
    clippy::suboptimal_flops
)]
fn oklch_to_linear_srgb(lightness: f32, chroma: f32, hue_degrees: f32) -> [f32; 3] {
    let hue_radians = hue_degrees.to_radians();
    let (sin_hue, cos_hue) = hue_radians.sin_cos();
    let a = chroma * cos_hue;
    let b = chroma * sin_hue;

    let l_ = lightness + 0.396_337_777_4_f32.mul_add(a, 0.215_803_757_3 * b);
    let m_ = lightness - 0.105_561_345_8_f32.mul_add(a, 0.063_854_172_8 * b);
    let s_ = lightness - 0.089_484_177_5_f32.mul_add(a, 1.291_485_548 * b);

    let l = l_.powi(3);
    let m = m_.powi(3);
    let s = s_.powi(3);

    [
        4.076_741_662_1_f32.mul_add(l, (-3.307_711_591_3_f32).mul_add(m, 0.230_969_929_2 * s)),
        (-1.268_438_004_6_f32).mul_add(l, 2.609_757_401_1_f32.mul_add(m, -0.341_319_396_5 * s)),
        (-0.004_196_086_3_f32).mul_add(l, (-0.703_418_614_7_f32).mul_add(m, 1.707_614_701 * s)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPSILON: f32 = 1e-5;
    const EPSILON_WIDE: f32 = 1e-3;

    fn approx_eq(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn srgb_linear_roundtrip() {
        let samples = [-0.25_f32, 0.0, 0.001, 0.02, 0.25, 0.5, 1.0, 1.25];

        for value in samples {
            let linear = srgb_to_linear(value);
            let recon = linear_to_srgb(linear);
            assert!(
                approx_eq(value, recon, EPSILON),
                "value {value} recon {recon}"
            );
        }
    }

    #[test]
    fn srgb_to_p3_and_back() {
        let samples = [
            Srgb::new(0.0, 0.0, 0.0),
            Srgb::new(0.25, 0.5, 0.75),
            Srgb::new(0.9, 0.2, 0.1),
            Srgb::new(0.6, 0.8, 0.1),
        ];

        for color in samples {
            let roundtrip = color.to_p3().to_srgb();
            assert!(approx_eq(color.red, roundtrip.red, EPSILON_WIDE));
            assert!(approx_eq(color.green, roundtrip.green, EPSILON_WIDE));
            assert!(approx_eq(color.blue, roundtrip.blue, EPSILON_WIDE));
        }
    }

    #[test]
    fn p3_to_srgb_and_back() {
        let samples = [
            P3::new(0.0, 0.0, 0.0),
            P3::new(0.3, 0.5, 0.7),
            P3::new(1.0, 0.0, 0.0),
            P3::new(0.2, 0.9, 0.3),
        ];

        for color in samples {
            let roundtrip = color.to_srgb().to_p3();
            assert!(approx_eq(color.red, roundtrip.red, EPSILON_WIDE));
            assert!(approx_eq(color.green, roundtrip.green, EPSILON_WIDE));
            assert!(approx_eq(color.blue, roundtrip.blue, EPSILON_WIDE));
        }
    }

    #[test]
    fn srgb_resolve_matches_linear_components() {
        let color = Srgb::from_hex("#4CAF50");
        let resolved = color.resolve();

        assert!(approx_eq(resolved.red, srgb_to_linear(color.red), EPSILON));
        assert!(approx_eq(
            resolved.green,
            srgb_to_linear(color.green),
            EPSILON
        ));
        assert!(approx_eq(
            resolved.blue,
            srgb_to_linear(color.blue),
            EPSILON
        ));
        assert!(approx_eq(resolved.headroom, 0.0, EPSILON));
        assert!(approx_eq(resolved.opacity, 1.0, EPSILON));
    }

    #[test]
    fn color_with_opacity_and_headroom_resolves() {
        let env = Environment::new();
        let base = Color::srgb(32, 64, 128)
            .with_opacity(0.4)
            .with_headroom(0.6);

        let resolved = base.resolve(&env).get();

        assert!(approx_eq(resolved.opacity, 0.4, EPSILON));
        assert!(approx_eq(resolved.headroom, 0.6, EPSILON));
    }

    #[test]
    fn p3_resolution_matches_conversion() {
        let env = Environment::new();
        let color = Color::p3(0.3, 0.6, 0.9);
        let resolved = color.resolve(&env).get();
        let srgb = P3::new(0.3, 0.6, 0.9).to_srgb().resolve();

        assert!(approx_eq(resolved.red, srgb.red, EPSILON_WIDE));
        assert!(approx_eq(resolved.green, srgb.green, EPSILON_WIDE));
        assert!(approx_eq(resolved.blue, srgb.blue, EPSILON_WIDE));
    }

    #[test]
    fn oklch_resolves_consistently() {
        let env = Environment::new();
        let samples = [
            Oklch::new(0.5, 0.1, 45.0),
            Oklch::new(0.75, 0.2, 200.0),
            Oklch::new(0.65, 0.05, 320.0),
        ];

        for sample in samples {
            let resolved_oklch = Color::from(sample).resolve(&env).get();
            let resolved_srgb = sample.to_srgb().resolve();

            assert!(approx_eq(
                resolved_oklch.red,
                resolved_srgb.red,
                EPSILON_WIDE
            ));
            assert!(approx_eq(
                resolved_oklch.green,
                resolved_srgb.green,
                EPSILON_WIDE
            ));
            assert!(approx_eq(
                resolved_oklch.blue,
                resolved_srgb.blue,
                EPSILON_WIDE
            ));
        }
    }

    #[test]
    fn hex_parsing_accepts_prefixes() {
        let direct = Srgb::from_hex("#1A2B3C");
        let prefixed = Srgb::from_hex("0x1A2B3C");
        let bare = Srgb::from_hex("1A2B3C");

        assert!(approx_eq(direct.red, prefixed.red, EPSILON));
        assert!(approx_eq(direct.green, prefixed.green, EPSILON));
        assert!(approx_eq(direct.blue, prefixed.blue, EPSILON));

        assert!(approx_eq(direct.red, bare.red, EPSILON));
        assert!(approx_eq(direct.green, bare.green, EPSILON));
        assert!(approx_eq(direct.blue, bare.blue, EPSILON));
    }

    #[test]
    fn try_hex_reports_errors() {
        assert!(matches!(
            Srgb::try_from_hex("#GGGGGG"),
            Err(HexColorError::InvalidDigit(1))
        ));

        assert!(matches!(
            Srgb::try_from_hex("#123"),
            Err(HexColorError::InvalidLength)
        ));
    }

    #[test]
    fn transparent_color_has_zero_opacity() {
        let env = Environment::new();
        let transparent = Color::transparent().resolve(&env).get();
        assert!(approx_eq(transparent.opacity, 0.0, EPSILON));
    }

    #[test]
    fn lighten_and_darken_adjust_lightness() {
        let env = Environment::new();
        let base = Color::oklch(0.4, 0.12, 90.0);
        let base_lch = base.resolve(&env).get().to_oklch();
        let lighter = base.clone().lighten(0.2).resolve(&env).get().to_oklch();
        let darker = base.darken(0.2).resolve(&env).get().to_oklch();

        assert!(lighter.lightness > base_lch.lightness);
        assert!(darker.lightness < base_lch.lightness);
    }

    #[test]
    fn saturate_and_desaturate_adjust_chroma() {
        let env = Environment::new();
        let base = Color::oklch(0.5, 0.2, 45.0);
        let base_chroma = base.resolve(&env).get().to_oklch().chroma;
        let saturated = base.clone().saturate(0.5).resolve(&env).get().to_oklch();
        let desaturated = base.desaturate(0.5).resolve(&env).get().to_oklch();

        assert!(saturated.chroma > base_chroma);
        assert!(desaturated.chroma < base_chroma);
    }

    #[test]
    fn hue_rotation_wraps_within_range() {
        let env = Environment::new();
        let rotated = Color::oklch(0.6, 0.18, 350.0)
            .hue_rotate(40.0)
            .resolve(&env)
            .get()
            .to_oklch();

        assert!(approx_eq(rotated.hue, 30.0, EPSILON_WIDE));
    }

    #[test]
    fn color_mixing_linearly_interpolates() {
        let env = Environment::new();
        let black = Color::srgb(0, 0, 0);
        let white = Color::srgb(255, 255, 255);
        let mid = black.mix(white, 0.5).resolve(&env).get();

        assert!(approx_eq(mid.red, 0.5, EPSILON));
        assert!(approx_eq(mid.green, 0.5, EPSILON));
        assert!(approx_eq(mid.blue, 0.5, EPSILON));
    }
}
