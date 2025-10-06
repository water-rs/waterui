//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports both sRGB and P3 color spaces, with utilities for conversion and
//! manipulation of color values.
//!
//! The primary type is `Color`, which can represent colors in either sRGB or P3
//! color spaces, with conversion methods from various tuple formats.

use core::{
    fmt::Debug,
    ops::{Deref, DerefMut},
};

use nami::{Computed, Signal, SignalExt, impl_constant};

use waterui_core::{
    Environment, raw_view,
    resolve::{self, AnyResolvable, Resolvable},
};

/// A color value that can be resolved in different color spaces.
///
/// This is the main color type that wraps a resolvable color value.
/// Colors can be created from sRGB, P3, or custom color spaces.
#[derive(Debug, Clone)]
pub struct Color(AnyResolvable<ResolvedColor>);

impl Default for Color {
    fn default() -> Self {
        Self::srgb(0, 0, 0)
    }
}

/// Represents a color in the Display P3 color space.
///
/// P3 is a wider color gamut than sRGB, commonly used in modern displays.
/// Component values are in the range 0.0 to 1.0.
#[derive(Debug, Clone, Copy)]
pub struct P3 {
    /// Red component (0.0 to 1.0)
    pub red: f32,
    /// Green component (0.0 to 1.0)
    pub green: f32,
    /// Blue component (0.0 to 1.0)
    pub blue: f32,
}

impl P3 {
    /// Creates a new P3 color from red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue }
    }

    /// Converts this P3 color to the sRGB color space.
    #[must_use]
    pub fn to_srgb(&self) -> Srgb {
        // convert p3 to srgb color space
        let linear = [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
        ];
        let srgb_linear = p3_to_linear_srgb(linear);
        Srgb::new(
            linear_to_srgb(srgb_linear[0]),
            linear_to_srgb(srgb_linear[1]),
            linear_to_srgb(srgb_linear[2]),
        )
    }
}

impl_constant!(ResolvedColor);

impl Resolvable for P3 {
    type Resolved = ResolvedColor;
    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let linear_p3 = [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
        ];
        let linear_srgb = p3_to_linear_srgb(linear_p3);
        ResolvedColor {
            red: linear_srgb[0],
            green: linear_srgb[1],
            blue: linear_srgb[2],
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}

impl<T: Resolvable<Resolved = ResolvedColor> + 'static> From<T> for Color {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

/// Represents a color in the sRGB color space.
///
/// sRGB is the standard RGB color space used in most displays and web content.
/// Component values are in the range 0.0 to 1.0.
#[derive(Debug, Clone, Copy)]
pub struct Srgb {
    /// Red component (0.0 to 1.0)
    pub red: f32,
    /// Green component (0.0 to 1.0)
    pub green: f32,
    /// Blue component (0.0 to 1.0)
    pub blue: f32,
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

mod parse {
    const fn hex_digit(b: u8) -> u8 {
        match b {
            b'0'..=b'9' => b - b'0',
            b'a'..=b'f' => b - b'a' + 10,
            b'A'..=b'F' => b - b'A' + 10,
            _ => panic!("invalid hex digit"),
        }
    }

    const fn from_hex_byte(s: &[u8], i: usize) -> u8 {
        (hex_digit(s[i]) << 4) | hex_digit(s[i + 1])
    }

    pub const fn parse_hex_color(s: &str) -> (u8, u8, u8) {
        let bytes = s.as_bytes();
        let mut i = 0;

        if !bytes.is_empty() && bytes[0] == b'#' {
            i = 1;
        } else if bytes.len() >= 2 && bytes[0] == b'0' && (bytes[1] == b'x' || bytes[1] == b'X') {
            i = 2;
        }

        if bytes.len() - i == 6 {
            (
                from_hex_byte(bytes, i),
                from_hex_byte(bytes, i + 2),
                from_hex_byte(bytes, i + 4),
            )
        } else {
            panic!("expected 6 hex digits");
        }
    }
}

impl Srgb {
    const RED: Self = Self::from_hex("#F44336");
    const PINK: Self = Self::from_hex("#E91E63");
    const PURPLE: Self = Self::from_hex("#9C27B0");
    const DEEP_PURPLE: Self = Self::from_hex("#673AB7");
    const INDIGO: Self = Self::from_hex("#3F51B5");
    const BLUE: Self = Self::from_hex("#2196F3");
    const LIGHT_BLUE: Self = Self::from_hex("#03A9F4");
    const CYAN: Self = Self::from_hex("#00BCD4");
    const TEAL: Self = Self::from_hex("#009688");
    const GREEN: Self = Self::from_hex("#4CAF50");
    const LIGHT_GREEN: Self = Self::from_hex("#8BC34A");
    const LIME: Self = Self::from_hex("#CDDC39");
    const YELLOW: Self = Self::from_hex("#FFEB3B");
    const AMBER: Self = Self::from_hex("#FFC107");
    const ORANGE: Self = Self::from_hex("#FF9800");
    const DEEP_ORANGE: Self = Self::from_hex("#FF5722");
    const BROWN: Self = Self::from_hex("#795548");
    const GREY: Self = Self::from_hex("#9E9E9E");
    const BLUE_GREY: Self = Self::from_hex("#607D8B");

    /// Creates a new sRGB color from red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue }
    }

    /// Creates a new sRGB color from 8-bit red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0-255)
    /// * `green` - Green component (0-255)
    /// * `blue` - Blue component (0-255)
    #[must_use]
    pub const fn new_u8(red: u8, green: u8, blue: u8) -> Self {
        Self {
            red: red as f32 / 255.0,
            green: green as f32 / 255.0,
            blue: blue as f32 / 255.0,
        }
    }

    /// Creates a new sRGB color from a hexadecimal color string.
    ///
    /// # Arguments
    /// * `hex` - Hex color string (e.g., "#FF5722" or "0xFF5722")
    #[must_use]
    pub const fn from_hex(hex: &str) -> Self {
        let (red, green, blue) = parse::parse_hex_color(hex);
        Self::new_u8(red, green, blue)
    }

    /// Converts this sRGB color to the P3 color space.
    #[must_use]
    pub fn to_p3(&self) -> P3 {
        // convert srgb to p3 color space
        let linear = [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
        ];
        let p3_linear = linear_srgb_to_p3(linear);
        P3::new(
            linear_to_srgb(p3_linear[0]),
            linear_to_srgb(p3_linear[1]),
            linear_to_srgb(p3_linear[2]),
        )
    }

    /// Creates a color with the specified opacity applied.
    ///
    /// # Arguments
    /// * `opacity` - Opacity value (0.0 = transparent, 1.0 = opaque)
    #[must_use]
    pub const fn with_opacity(self, opacity: f32) -> WithOpacity<Self> {
        WithOpacity::new(self, opacity)
    }

    /// Resolves this sRGB color to a `ResolvedColor` in linear RGB color space.
    #[must_use]
    pub fn resolve(&self) -> ResolvedColor {
        ResolvedColor {
            red: srgb_to_linear(self.red),
            green: srgb_to_linear(self.green),
            blue: srgb_to_linear(self.blue),
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}

impl Resolvable for Srgb {
    type Resolved = ResolvedColor;
    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        ResolvedColor {
            red: srgb_to_linear(self.red),
            green: srgb_to_linear(self.green),
            blue: srgb_to_linear(self.blue),
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}

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

#[derive(Debug, Default, Clone, Copy, PartialEq, PartialOrd, Hash, Eq, Ord)]
#[non_exhaustive]
/// Represents the supported color spaces for color representation.
pub enum Colorspace {
    /// Standard RGB color space (sRGB) with values typically in the range 0-255.
    #[default]
    Srgb,
    /// Display P3 color space with extended color gamut, using floating-point values 0.0-1.0.
    P3,
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

    /// Creates a new color with the specified opacity applied.
    ///
    /// # Arguments
    /// * `opacity` - Opacity value (0.0 = transparent, 1.0 = opaque)
    #[must_use]
    pub fn with_opacity(self, opacity: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut resolved| {
            resolved.opacity = opacity;
            resolved
        }))
    }

    /// Creates a new color with extended headroom for HDR content.
    ///
    /// # Arguments
    /// * `headroom` - Additional headroom value for extended range
    #[must_use]
    pub fn with_headroom(self, headroom: f32) -> Self {
        Self::new(resolve::Map::new(self.0, move |mut resolved| {
            resolved.headroom = headroom;
            resolved
        }))
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
raw_view!(Color); // should be filled rectangle

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
}
