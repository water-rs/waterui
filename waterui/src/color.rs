use core::ops::Add;

use color::{AlphaColor, DynamicColor, Flags, Srgb};
use waterui_core::{Environment, View};
use waterui_reactive::{compute::IntoComputed, zip::FlattenMap, Compute, ComputeExt, Computed};

use crate::{component::shape::Rectangle, ViewExt};

#[derive(Debug, Clone, PartialEq)]
pub struct Color(DynamicColor);

impl Default for Color {
    fn default() -> Self {
        todo!()
    }
}
impl Color {
    pub fn srgb(red: f32, green: f32, blue: f32) -> Self {
        Self(DynamicColor::from_alpha_color(AlphaColor::<Srgb>::new([
            red, green, blue, 1.0,
        ])))
    }

    pub fn p3(red: f32, green: f32, blue: f32) -> Self {
        // DynamicColor::from_alpha_color(AlphaColor::new([red, green, blue]))
        todo!()
    }

    /*  pub const fn with_space(mut self, space: ColorSpace) -> Self {
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
    }*/
}

#[derive(Debug)]
pub struct BackgroundColor {
    pub color: Computed<Color>,
}

impl BackgroundColor {
    pub fn new(color: impl IntoComputed<Color>) -> Self {
        Self {
            color: color.into_computed(),
        }
    }
}

#[derive(Debug)]
pub struct ForegroundColor {
    pub color: Computed<Color>,
}

impl ForegroundColor {
    pub fn new(color: impl IntoComputed<Color>) -> Self {
        Self {
            color: color.into_computed(),
        }
    }
}

impl View for Color {
    fn body(self, _env: &Environment) -> impl View {
        Rectangle.foreground(self)
    }
}

impl Add for Color {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        //self.mix(rhs, 0.5)
        todo!()
    }
}

pub fn mix(
    left: impl Compute<Output = Color> + 'static,
    right: impl Compute<Output = Color> + 'static,
) -> impl Compute<Output = Color> {
    left.zip(right).flatten_map(|left, right| left + right)
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
