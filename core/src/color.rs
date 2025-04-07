use core::ops::Add;

use crate::{Environment, View};

use crate::shape::Rectangle;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Srgb {
    red: u8,
    yellow: u8,
    blue: u8,
}

#[derive(Debug, Clone, PartialEq, PartialOrd, Default)]
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
