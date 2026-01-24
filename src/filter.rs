//! # Filters Module
//!
//! This module provides a collection of visual filters that can be applied to views.
//! Filters allow for visual effects such as blurring, color adjustments, and other
//! transformations to be applied to the rendered content.
//!
//! Each filter is represented by a structure with reactive values that can be animated.
//! Filters are purely visual and do not affect layout calculations.

use nami::Computed;
use waterui_core::{IntoComputedF32, metadata::MetadataKey};

/// A structure representing a blur filter operation.
///
/// Applies a Gaussian blur to the view content.
#[derive(Debug)]
pub struct Blur {
    /// The radius of the blur effect in points.
    /// 0 means no blur, higher values increase blur intensity.
    pub radius: Computed<f32>,
}

impl MetadataKey for Blur {}

impl Blur {
    /// Creates a new blur filter with the specified radius.
    ///
    /// # Arguments
    ///
    /// * `radius` - The radius of the blur in points (0 = no blur).
    #[must_use]
    pub fn new(radius: impl IntoComputedF32) -> Self {
        Self {
            radius: radius.into_computed_f32(),
        }
    }
}

/// A structure representing a brightness adjustment filter.
///
/// Adjusts the overall brightness of the view content.
#[derive(Debug)]
pub struct Brightness {
    /// The amount of brightness adjustment.
    /// 0 is normal brightness, negative values darken, positive values brighten.
    /// Range: typically -1.0 to 1.0.
    pub amount: Computed<f32>,
}

impl MetadataKey for Brightness {}

impl Brightness {
    /// Creates a new brightness filter with the specified amount.
    ///
    /// # Arguments
    ///
    /// * `amount` - The brightness adjustment amount.
    ///   0 is normal brightness, negative darkens, positive brightens.
    #[must_use]
    pub fn new(amount: impl IntoComputedF32) -> Self {
        Self {
            amount: amount.into_computed_f32(),
        }
    }
}

/// A structure representing a contrast adjustment filter.
///
/// Adjusts the contrast of the view content.
#[derive(Debug)]
pub struct Contrast {
    /// The amount of contrast adjustment.
    /// 1.0 is normal contrast, values below 1.0 decrease contrast,
    /// values above 1.0 increase contrast.
    pub amount: Computed<f32>,
}

impl MetadataKey for Contrast {}

impl Contrast {
    /// Creates a new contrast filter with the specified amount.
    ///
    /// # Arguments
    ///
    /// * `amount` - The contrast adjustment amount.
    ///   1.0 is normal contrast, <1.0 decreases, >1.0 increases.
    #[must_use]
    pub fn new(amount: impl IntoComputedF32) -> Self {
        Self {
            amount: amount.into_computed_f32(),
        }
    }
}

/// A structure representing a saturation adjustment filter.
///
/// Adjusts the color saturation of the view content.
#[derive(Debug)]
pub struct Saturation {
    /// The amount of saturation adjustment.
    /// 1.0 is normal saturation, 0 is grayscale, values above 1.0 oversaturate.
    pub amount: Computed<f32>,
}

impl MetadataKey for Saturation {}

impl Saturation {
    /// Creates a new saturation filter with the specified amount.
    ///
    /// # Arguments
    ///
    /// * `amount` - The saturation adjustment amount.
    ///   1.0 is normal, 0 is grayscale, >1.0 oversaturates.
    #[must_use]
    pub fn new(amount: impl IntoComputedF32) -> Self {
        Self {
            amount: amount.into_computed_f32(),
        }
    }
}

/// A structure representing a grayscale filter.
///
/// Converts the view content to grayscale.
#[derive(Debug)]
pub struct Grayscale {
    /// The intensity of the grayscale effect.
    /// 0.0 means no effect (full color), 1.0 means full grayscale.
    pub intensity: Computed<f32>,
}

impl MetadataKey for Grayscale {}

impl Grayscale {
    /// Creates a new grayscale filter with the specified intensity.
    ///
    /// # Arguments
    ///
    /// * `intensity` - The intensity of the grayscale effect.
    ///   0.0 means no effect, 1.0 means full grayscale.
    #[must_use]
    pub fn new(intensity: impl IntoComputedF32) -> Self {
        Self {
            intensity: intensity.into_computed_f32(),
        }
    }
}

/// A structure representing a hue rotation filter.
///
/// Rotates the hue of all colors in the view content.
#[derive(Debug)]
pub struct HueRotation {
    /// The angle of rotation in degrees.
    /// 0 means no rotation, 180 inverts colors, 360 is a full rotation.
    pub angle: Computed<f32>,
}

impl MetadataKey for HueRotation {}

impl HueRotation {
    /// Creates a new hue rotation filter with the specified angle.
    ///
    /// # Arguments
    ///
    /// * `angle` - The angle of hue rotation in degrees (0-360).
    #[must_use]
    pub fn new(angle: impl IntoComputedF32) -> Self {
        Self {
            angle: angle.into_computed_f32(),
        }
    }
}

/// A structure representing an opacity filter.
///
/// Adjusts the transparency of the view content.
#[derive(Debug)]
pub struct Opacity {
    /// The opacity value.
    /// 0.0 is fully transparent, 1.0 is fully opaque.
    pub value: Computed<f32>,
}

impl MetadataKey for Opacity {}

impl Opacity {
    /// Creates a new opacity filter with the specified value.
    ///
    /// # Arguments
    ///
    /// * `value` - The opacity value (0.0 = transparent, 1.0 = opaque).
    #[must_use]
    pub fn new(value: impl IntoComputedF32) -> Self {
        Self {
            value: value.into_computed_f32(),
        }
    }
}
