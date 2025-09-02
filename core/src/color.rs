//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports both sRGB and P3 color spaces, with utilities for conversion and
//! manipulation of color values.
//!
//! The primary type is `Color`, which can represent colors in either sRGB or P3
//! color spaces, with conversion methods from various tuple formats.

use nami::impl_constant;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]

/// Represents an sRGB color with red, green, and blue components.
pub struct Srgb {
    red: u8,
    green: u8,
    blue: u8,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]

/// Represents a P3 color with red, green, and blue components.
pub struct P3 {
    red: f32,
    green: f32,
    blue: f32,
}

#[derive(Debug, Clone, PartialEq, PartialOrd)]
enum ColorInner {
    Srgb(Srgb),
    P3(P3),
}

impl Default for ColorInner {
    fn default() -> Self {
        Self::Srgb(Srgb::default())
    }
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
/// Represents a color, either in sRGB or P3 color space.
pub struct Color {
    color: ColorInner,
    opacity: f32,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Hash, Eq, Ord)]
#[non_exhaustive]
pub enum Colorspace {
    Srgb,
    P3,
}

impl_constant!(Color);

impl From<(u8, u8, u8)> for Color {
    fn from((red, green, blue): (u8, u8, u8)) -> Self {
        Self {
            color: ColorInner::Srgb(Srgb { red, green, blue }),
            opacity: 1.0,
        }
    }
}

impl From<(f32, f32, f32)> for Color {
    fn from((red, green, blue): (f32, f32, f32)) -> Self {
        Self {
            color: ColorInner::P3(P3 { red, green, blue }),
            opacity: 1.0,
        }
    }
}

impl From<(f32, f32, f32, f32)> for Color {
    fn from((red, green, blue, opacity): (f32, f32, f32, f32)) -> Self {
        Self {
            color: ColorInner::P3(P3 { red, green, blue }),
            opacity,
        }
    }
}

impl Color {
    /// Get the RGBA values as f32 (0.0 to 1.0 range)
    #[must_use]
    pub fn rgba(&self) -> (f32, f32, f32, f32) {
        match &self.color {
            ColorInner::Srgb(srgb) => (
                f32::from(srgb.red) / 255.0,
                f32::from(srgb.green) / 255.0,
                f32::from(srgb.blue) / 255.0,
                self.opacity,
            ),
            ColorInner::P3(p3) => (p3.red, p3.green, p3.blue, self.opacity),
        }
    }

    /// Get the sRGB values as u8 (0 to 255 range)
    #[must_use]
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn srgb_u8(&self) -> (u8, u8, u8, u8) {
        let (r, g, b, a) = self.rgba();
        (
            (r * 255.0) as u8,
            (g * 255.0) as u8,
            (b * 255.0) as u8,
            (a * 255.0) as u8,
        )
    }

    pub fn p3(&self) -> (f32, f32, f32, f32) {
        match &self.color {
            ColorInner::P3(p3) => (p3.red, p3.green, p3.blue, self.opacity),
            ColorInner::Srgb(srgb) => (
                f32::from(srgb.red) / 255.0,
                f32::from(srgb.green) / 255.0,
                f32::from(srgb.blue) / 255.0,
                self.opacity,
            ),
        }
    }

    /// Create a Color from RGBA f32 values (0.0 to 1.0)
    #[must_use]
    pub const fn from_rgba(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            color: ColorInner::P3(P3 { red, green, blue }),
            opacity: alpha,
        }
    }

    pub const fn color_space(&self) -> Colorspace {
        match &self.color {
            ColorInner::Srgb(_) => Colorspace::Srgb,
            ColorInner::P3(_) => Colorspace::P3,
        }
    }
}

raw_view!(Color);
