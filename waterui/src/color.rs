use core::ops::Add;

use waterui_core::View;
use waterui_reactive::{compute::ToComputed, impl_constant, Compute, ComputeExt, Computed};

use crate::{component::shape::Rectangle, ViewExt};

#[derive(Debug, Clone, PartialEq, PartialOrd)]
#[repr(C)]
pub struct Color {
    pub space: ColorSpace,
    pub red: f64,
    pub green: f64,
    pub blue: f64,
    pub opacity: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(C)]
pub enum ColorSpace {
    sRGB,
    P3,
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

pub mod md2 {
    use super::Color;

    pub const RED: Color = Color::rgb(244.0, 67.0, 54.0);
    pub const PURPLE: Color = Color::rgb(156.0, 39.0, 176.0);
    pub const BLUE: Color = Color::rgb(33.0, 150.0, 243.0);
}

impl Color {
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(255.0, 255.0, 255.0);
    pub const fn rgb(red: f64, green: f64, blue: f64) -> Self {
        Self {
            space: ColorSpace::sRGB,
            red,
            green,
            blue,
            opacity: 1.0,
        }
    }

    pub fn hsb(hue: f64, saturation: f64, brightness: f64) -> Self {
        let (red, green, blue) = hsb_to_rgb(hue, saturation, brightness);
        Self::rgb(red, green, blue)
    }

    pub const fn with_space(mut self, space: ColorSpace) -> Self {
        self.space = space;
        self
    }

    pub const fn with_opacity(mut self, opacity: f64) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn mix(&self, other: Self, weight: f64) -> Self {
        assert!((0.0..=1.0).contains(&weight));
        Self::rgb(
            (self.red + other.red) * weight,
            (self.green + other.green) * weight,
            (self.blue + other.blue) * weight,
        )
    }
}

#[derive(Debug)]
pub struct BackgroundColor {
    pub color: Computed<Color>,
}

impl BackgroundColor {
    pub fn new(color: impl ToComputed<Color>) -> Self {
        Self {
            color: color.to_computed(),
        }
    }
}

#[derive(Debug)]
pub struct ForegroundColor {
    pub color: Computed<Color>,
}

impl ForegroundColor {
    pub fn new(color: impl ToComputed<Color>) -> Self {
        Self {
            color: color.to_computed(),
        }
    }
}

impl View for Color {
    fn body(self, _env: waterui_core::Environment) -> impl View {
        Rectangle.foreground(self)
    }
}

impl_constant!(Color);

impl Add for Color {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        self.mix(rhs, 0.5)
    }
}

pub fn mix(
    left: impl Compute<Output = Color> + 'static,
    right: impl Compute<Output = Color> + 'static,
) -> impl Compute<Output = Color> {
    (left, right).map(|(left, right)| left + right)
}

fn hsb_to_rgb(h: f64, s: f64, b: f64) -> (f64, f64, f64) {
    let h = h % 360.0;
    let c = b * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());

    let (r1, g1, b1) = match h_prime as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        5 => (c, 0.0, x),
        _ => (0.0, 0.0, 0.0),
    };

    let m = b - c;

    (r1 + m, g1 + m, b1 + m)
}
