//! # Color Module
//!
//! This module provides types for working with colors in different color spaces.
//! It supports both sRGB and P3 color spaces, with utilities for conversion and
//! manipulation of color values.
//!
//! The primary type is `Color`, which can represent colors in either sRGB or P3
//! color spaces, with conversion methods from various tuple formats.

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
/// Represents an sRGB color with red, yellow, and blue components.
pub struct Srgb {
    red: u8,
    yellow: u8,
    blue: u8,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
/// Represents a P3 color with red, yellow, blue, and opacity components.
pub struct P3 {
    red: f32,
    yellow: f32,
    blue: f32,
    opacity: f32,
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
pub struct Color(ColorInner);

impl From<(u8, u8, u8)> for Color {
    fn from((red, yellow, blue): (u8, u8, u8)) -> Self {
        Self(ColorInner::Srgb(Srgb { red, yellow, blue }))
    }
}

impl From<(f32, f32, f32)> for Color {
    fn from((red, yellow, blue): (f32, f32, f32)) -> Self {
        Self(ColorInner::P3(P3 {
            red,
            yellow,
            blue,
            opacity: 1.0,
        }))
    }
}

impl From<(f32, f32, f32, f32)> for Color {
    fn from((red, yellow, blue, opacity): (f32, f32, f32, f32)) -> Self {
        Self(ColorInner::P3(P3 {
            red,
            yellow,
            blue,
            opacity,
        }))
    }
}

raw_view!(Color);
