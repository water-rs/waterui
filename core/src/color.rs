//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports both sRGB and P3 color spaces, with utilities for conversion and
//! manipulation of color values.
//!
//! The primary type is `Color`, which can represent colors in either sRGB or P3
//! color spaces, with conversion methods from various tuple formats.

#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
/// Represents an sRGB color with red, yellow, and blue components.
pub struct Srgb {
    red: u8,
    yellow: u8,
    blue: u8,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
/// Represents a P3 color with red, yellow, and blue components.
pub struct P3 {
    red: f32,
    yellow: f32,
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

impl From<(u8, u8, u8)> for Color {
    fn from((red, yellow, blue): (u8, u8, u8)) -> Self {
        Self {
            color: ColorInner::Srgb(Srgb { red, yellow, blue }),
            opacity: 1.0,
        }
    }
}

impl From<(f32, f32, f32)> for Color {
    fn from((red, yellow, blue): (f32, f32, f32)) -> Self {
        Self {
            color: ColorInner::P3(P3 { red, yellow, blue }),
            opacity: 1.0,
        }
    }
}

impl From<(f32, f32, f32, f32)> for Color {
    fn from((red, yellow, blue, opacity): (f32, f32, f32, f32)) -> Self {
        Self {
            color: ColorInner::P3(P3 { red, yellow, blue }),
            opacity,
        }
    }
}

raw_view!(Color);

pub(crate) mod ffi {
    use waterui_ffi::{IntoFFI, IntoNullableFFI};

    use super::{Color, ColorInner};

    #[repr(C)]
    pub enum WuiColorSpace {
        Srgb,
        P3,
        Invalid,
    }

    #[repr(C)]
    pub struct WuiColor {
        color_space: WuiColorSpace,
        red: f32,
        yellow: f32,
        blue: f32,
        opacity: f32,
    }

    impl IntoNullableFFI for Color {
        type FFI = WuiColor;
        fn into_ffi(self) -> Self::FFI {
            match self.color {
                ColorInner::Srgb(srgb) => WuiColor {
                    color_space: WuiColorSpace::Srgb,
                    red: srgb.red as f32,
                    yellow: srgb.yellow as f32,
                    blue: srgb.blue as f32,
                    opacity: self.opacity,
                },
                ColorInner::P3(p3) => WuiColor {
                    color_space: WuiColorSpace::P3,
                    red: p3.red,
                    yellow: p3.yellow,
                    blue: p3.blue,
                    opacity: self.opacity,
                },
            }
        }

        fn null() -> Self::FFI {
            WuiColor {
                color_space: WuiColorSpace::Invalid,
                red: 0.0,
                yellow: 0.0,
                blue: 0.0,
                opacity: 0.0,
            }
        }
    }
}
