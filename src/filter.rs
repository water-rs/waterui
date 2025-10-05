//! # Filters Module
//!
//! This module provides a collection of visual filters that can be applied to views.
//! Filters allow for visual effects such as blurring, color adjustments, and other
//! transformations to be applied to the rendered content.
//!
//! Each filter is represented by a structure that can be configured and applied to
//! a view to achieve the desired visual effect.

/// A structure representing a blur filter operation.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Blur {
    /// The radius of the blur effect in pixels.
    pub radius: f32,
}

impl Blur {
    /// Creates a new blur filter with the specified radius.
    ///
    /// # Arguments
    ///
    /// * `radius` - The radius of the blur in pixels.
    #[must_use]
    pub const fn new(radius: f32) -> Self {
        Self { radius }
    }
}

/// A structure representing a brightness adjustment filter.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Brightness {
    /// The amount of brightness adjustment.
    /// Values above 1.0 increase brightness, values below 1.0 decrease brightness.
    pub amount: f32,
}

impl Brightness {
    /// Creates a new brightness filter with the specified amount.
    ///
    /// # Arguments
    ///
    /// * `amount` - The brightness adjustment amount. 1.0 is normal brightness,
    ///   values above 1.0 increase brightness, values below 1.0 decrease brightness.
    #[must_use]
    pub const fn new(amount: f32) -> Self {
        Self { amount }
    }
}

/// A structure representing a contrast adjustment filter.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Contrast {
    /// The amount of contrast adjustment.
    /// Values above 1.0 increase contrast, values below 1.0 decrease contrast.
    pub amount: f32,
}

impl Contrast {
    /// Creates a new contrast filter with the specified amount.
    ///
    /// # Arguments
    ///
    /// * `amount` - The contrast adjustment amount. 1.0 is normal contrast,
    ///   values above 1.0 increase contrast, values below 1.0 decrease contrast.
    #[must_use]
    pub const fn new(amount: f32) -> Self {
        Self { amount }
    }
}

/// A structure representing a saturation adjustment filter.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Saturation {
    /// The amount of saturation adjustment.
    /// Values above 1.0 increase saturation, values below 1.0 decrease saturation.
    pub amount: f32,
}

impl Saturation {
    /// Creates a new saturation filter with the specified amount.
    ///
    /// # Arguments
    ///
    /// * `amount` - The saturation adjustment amount. 1.0 is normal saturation,
    ///   values above 1.0 increase saturation, values below 1.0 decrease saturation.
    #[must_use]
    pub const fn new(amount: f32) -> Self {
        Self { amount }
    }
}

/// A structure representing a grayscale filter.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Grayscale {
    /// The intensity of the grayscale effect.
    /// 0.0 means no effect, 1.0 means full grayscale.
    pub intensity: f32,
}

impl Grayscale {
    /// Creates a new grayscale filter with the specified intensity.
    ///
    /// # Arguments
    ///
    /// * `intensity` - The intensity of the grayscale effect.
    ///   0.0 means no effect, 1.0 means full grayscale.
    #[must_use]
    pub const fn new(intensity: f32) -> Self {
        Self { intensity }
    }
}

/// A structure representing a hue rotation filter.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct HueRotation {
    /// The angle of rotation in degrees.
    pub angle: f32,
}

impl HueRotation {
    /// Creates a new hue rotation filter with the specified angle.
    ///
    /// # Arguments
    ///
    /// * `angle` - The angle of hue rotation in degrees.
    #[must_use]
    pub const fn new(angle: f32) -> Self {
        Self { angle }
    }
}

/// A structure representing an inversion filter.
#[derive(Debug, Clone)]
#[repr(C)]
pub struct Invert {
    /// The intensity of the inversion effect.
    /// 0.0 means no effect, 1.0 means full inversion.
    pub intensity: f32,
}

impl Invert {
    /// Creates a new inversion filter with the specified intensity.
    ///
    /// # Arguments
    ///
    /// * `intensity` - The intensity of the inversion effect.
    ///   0.0 means no effect, 1.0 means full inversion.
    #[must_use]
    pub const fn new(intensity: f32) -> Self {
        Self { intensity }
    }
}
