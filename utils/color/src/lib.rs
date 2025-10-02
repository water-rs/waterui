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
        Color::new(Srgb::new(0, 0, 0, 0.0))
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
        ResolvedColor {
            red: self.red,
            green: self.green,
            blue: self.blue,
            headroom: 0.0,
            opacity: self.opacity,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Srgb {
    red: u8,
    green: u8,
    blue: u8,
    opacity: f32,
}

impl Srgb {
    pub fn new(red: u8, green: u8, blue: u8, opacity: f32) -> Self {
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
            red: self.red as f32 / 255.0,
            green: self.green as f32 / 255.0,
            blue: self.blue as f32 / 255.0,
            headroom: 0.0,
            opacity: self.opacity,
        }
    }
}

// Extended SRGB
#[derive(Debug, Clone, Copy)]
pub struct ResolvedColor {
    red: f32, // 0.0-1.0 for sRGB, <0 or >1 for P3
    green: f32,
    blue: f32,
    headroom: f32,
    opacity: f32,
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
