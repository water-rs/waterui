//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports both sRGB and P3 color spaces, with utilities for conversion and
//! manipulation of color values.
//!
//! The primary type is `Color`, which can represent colors in either sRGB or P3
//! color spaces, with conversion methods from various tuple formats.

use core::fmt::Debug;

use nami::impl_constant;

use waterui_core::{Environment, raw_view};

#[derive(Debug)]
pub struct Color(Box<dyn CustomColorImpl>);

impl Clone for Color {
    fn clone(&self) -> Self {
        Self(self.0.clone_box())
    }
}

impl Default for Color {
    fn default() -> Self {
        Color::srgb(0, 0, 0, 0.0)
    }
}

trait CustomColorImpl: Debug {
    fn resolve(&self, env: &Environment) -> ResolvedColor;
    fn clone_box(&self) -> Box<dyn CustomColorImpl>;
}

impl<T: CustomColor + Clone + 'static> CustomColorImpl for T {
    fn resolve(&self, env: &Environment) -> ResolvedColor {
        self.resolve(env)
    }

    fn clone_box(&self) -> Box<dyn CustomColorImpl> {
        Box::new(self.clone())
    }
}

pub trait CustomColor: Debug + Clone {
    fn resolve(&self, env: &Environment) -> ResolvedColor;
}

#[derive(Debug, Clone, Copy)]
pub struct P3 {
    red: f32,
    green: f32,
    blue: f32,
    opacity: f32,
}

impl P3 {
    pub fn new(red: f32, green: f32, blue: f32, opacity: f32) -> Self {
        Self {
            red,
            green,
            blue,
            opacity,
        }
    }
}

impl CustomColor for P3 {
    fn resolve(&self, _env: &Environment) -> ResolvedColor {
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
            opacity: self.opacity,
        }
    }
}

impl<T:CustomColor + 'static> From<T> for Color {
    fn from(value: T) -> Self {
        Self::new(value)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Srgb {
    red: f32,
    green: f32,
    blue: f32,
    opacity: f32,
}

impl Srgb {
    pub fn new(red: f32, green: f32, blue: f32, opacity: f32) -> Self {
        Self {
            red,
            green,
            blue,
            opacity,
        }
    }
}

impl CustomColor for Srgb {
    fn resolve(&self, _env: &Environment) -> ResolvedColor {
        ResolvedColor {
            red: srgb_to_linear(self.red),
            green: srgb_to_linear(self.green),
            blue: srgb_to_linear(self.blue),
            headroom: 0.0,
            opacity: self.opacity,
        }
    }
}

// Extended SRGB
#[derive(Debug, Clone, Copy)]
pub struct ResolvedColor {
    pub red: f32, // 0.0-1.0 for sRGB, <0 or >1 for P3
    pub green: f32,
    pub blue: f32,
    pub headroom: f32,
    pub opacity: f32,
}

pub struct WithOpacity {
    base: Color,
    opacity: f32,
}

pub struct WithColorSpace {
    base: Color,
    space: Colorspace,
}

pub struct WithHeadroom {
    base: Color,
    headroom: f32,
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
    pub fn new(custom: impl CustomColor + 'static) -> Self {
        Self(Box::new(custom))
    }

    pub fn srgb(red: u8, green: u8, blue: u8, opacity: f32) -> Self {
        Self::new(Srgb::new(
            red as f32 / 255.0,
            green as f32 / 255.0,
            blue as f32 / 255.0,
            opacity,
        ))
    }

    pub fn srgb_f32(red: f32, green: f32, blue: f32, opacity: f32) -> Self {
        Self::new(Srgb::new(red, green, blue, opacity))
    }

    pub fn p3(red: f32, green: f32, blue: f32, opacity: f32) -> Self {
        Self::new(P3::new(red, green, blue, opacity))
    }

    pub fn resolve(&self, env: &Environment) -> ResolvedColor {
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
        1.2249401 * p3[0] - 0.2249401 * p3[1],
        -0.0420301 * p3[0] + 1.0420301 * p3[1],
        -0.0197211 * p3[0] - 0.0786361 * p3[1] + 1.0983572 * p3[2],
    ]
}
