//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports both sRGB and P3 color spaces, with utilities for conversion and
//! manipulation of color values.
//!
//! The primary type is `Color`, which can represent colors in either sRGB or P3
//! color spaces, with conversion methods from various tuple formats.

use core::fmt::Debug;

use nami::{impl_constant, Computed, Signal, SignalExt};

use waterui_core::{raw_view, resolve::{self, AnyResolvable, Resolvable}, Environment};

/// A color value that can be resolved in different color spaces.
///
/// This is the main color type that wraps a resolvable color value.
/// Colors can be created from sRGB, P3, or custom color spaces.
#[derive(Debug,Clone)]
pub struct Color(AnyResolvable<ResolvedColor>);


impl Default for Color {
    fn default() -> Self {
        Self::srgb(0, 0, 0, 0.0)
    }
}



/// Represents a color in the Display P3 color space.
///
/// P3 is a wider color gamut than sRGB, commonly used in modern displays.
/// Component values are in the range 0.0 to 1.0.
#[derive(Debug, Clone, Copy)]
pub struct P3 {
    red: f32,
    green: f32,
    blue: f32,
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
        Self {
            red,
            green,
            blue,
        }
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
    red: f32,
    green: f32,
    blue: f32,
}

impl Srgb {
    /// Creates a new sRGB color from red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use] 
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self {
            red,
            green,
            blue,
        }
    }
    
    /// Converts this sRGB color to the P3 color space.
    #[must_use]
    pub fn to_p3(&self) -> P3{
        // convert srgb to p3 color space
        todo!()
    }

    /// Creates a color with the specified opacity applied.
    ///
    /// # Arguments
    /// * `opacity` - Opacity value (0.0 = transparent, 1.0 = opaque)
    pub fn with_opacity(self, opacity: f32) -> impl Resolvable<Resolved = ResolvedColor> + 'static {
        resolve::Map::new(self, move |mut resolved| {
            resolved.opacity = opacity;
            resolved
        })
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
    /// * `_opacity` - Opacity value (currently unused, reserved for future use)
    pub fn srgb(red: u8, green: u8, blue: u8, _opacity: f32) -> Self {
        Self::new(Srgb::new(
            red as f32 / 255.0,
            green as f32 / 255.0,
            blue as f32 / 255.0,
        ))
    }

    /// Creates an sRGB color from floating-point color components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    pub fn srgb_f32(red: f32, green: f32, blue: f32) -> Self {
        Self::new(Srgb::new(red, green, blue))
    }

    /// Creates a P3 color from floating-point color components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
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
    pub fn resolve(&self, env: &Environment) -> Computed<ResolvedColor> {
        self.0.resolve(env)
    }
}

raw_view!(Color); // should be filled rectangle

// https://www.w3.org/TR/css-color-4/#color-conversion-code
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

// Conversion matrix from P3 to sRGB
// https://www.w3.org/TR/css-color-4/#color-conversion-code
fn p3_to_linear_srgb(p3: [f32; 3]) -> [f32; 3] {
    [
        1.224_940_1_f32.mul_add(p3[0], -0.224_940_1 * p3[1]),
        (-0.042_030_1_f32).mul_add(p3[0], 1.042_030_1 * p3[1]),
        (-0.019_721_1_f32).mul_add(p3[0], (-0.078_636_1_f32).mul_add(p3[1], 1.098_357_2 * p3[2])),
    ]
}
