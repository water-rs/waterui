use nami::Signal;
use waterui_core::{Environment, resolve::Resolvable};

use crate::{ResolvedColor, Srgb, linear_to_srgb, oklch_to_linear_srgb};

/// Represents a color in the perceptually-uniform OKLCH color space.
///
/// Lightness is expressed in the range 0.0 to 1.0, chroma controls the color
/// intensity, and hue is measured in degrees.
#[derive(Debug, Clone, Copy)]
pub struct Oklch {
    /// Perceptual lightness component (0.0 to 1.0).
    pub lightness: f32,
    /// Perceptual chroma component.
    pub chroma: f32,
    /// Hue angle in degrees.
    pub hue: f32,
}

impl Oklch {
    /// Creates a new OKLCH color from its lightness, chroma, and hue
    /// components.
    #[must_use]
    pub const fn new(lightness: f32, chroma: f32, hue: f32) -> Self {
        Self {
            lightness,
            chroma,
            hue,
        }
    }

    /// Converts this OKLCH color into the sRGB color space.
    #[must_use]
    pub fn to_srgb(&self) -> Srgb {
        let [red, green, blue] = oklch_to_linear_srgb(self.lightness, self.chroma, self.hue);

        Srgb::new(
            linear_to_srgb(red),
            linear_to_srgb(green),
            linear_to_srgb(blue),
        )
    }
}

impl Resolvable for Oklch {
    type Resolved = ResolvedColor;

    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let [red, green, blue] = oklch_to_linear_srgb(self.lightness, self.chroma, self.hue);

        ResolvedColor {
            red,
            green,
            blue,
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}
