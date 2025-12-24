use nami::Signal;
use waterui_core::{Environment, resolve::Resolvable};

use crate::{ResolvedColor, Srgb, linear_to_srgb, p3_to_linear_srgb, srgb_to_linear};

/// Represents a color in the Display P3 color space.
///
/// P3 is a wider color gamut than sRGB, commonly used in modern displays.
/// Component values are in the range 0.0 to 1.0.
#[derive(Debug, Clone, Copy)]
pub struct P3 {
    /// Red component (0.0 to 1.0)
    pub red: f32,
    /// Green component (0.0 to 1.0)
    pub green: f32,
    /// Blue component (0.0 to 1.0)
    pub blue: f32,
}

impl P3 {
    /// Creates a new P3 color from red, green, and blue components.
    ///
    /// # Arguments
    /// * `red` - Red component (0.0 to 1.0)
    /// * `green` - Green component (0.0 to 1.0)
    /// * `blue` - Blue component (0.0 to 1.0)
    #[must_use]
    pub const fn new(red: f32, green: f32, blue: f32) -> Self {
        Self { red, green, blue }
    }

    /// Converts this P3 color to the sRGB color space.
    #[must_use]
    pub fn to_srgb(&self) -> Srgb {
        // convert p3 to srgb color space
        let linear = [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
        ];
        let srgb_linear = p3_to_linear_srgb(linear);
        Srgb::new(
            linear_to_srgb(srgb_linear[0]),
            linear_to_srgb(srgb_linear[1]),
            linear_to_srgb(srgb_linear[2]),
        )
    }
}

impl Resolvable for P3 {
    type Resolved = ResolvedColor;
    fn resolve(&self, _env: &Environment) -> impl Signal<Output = Self::Resolved> {
        let linear_p3 = [
            srgb_to_linear(self.red),
            srgb_to_linear(self.green),
            srgb_to_linear(self.blue),
        ];
        let linear_srgb = p3_to_linear_srgb(linear_p3);
        ResolvedColor {
            red: linear_srgb[0],
            green: linear_srgb[1],
            blue: linear_srgb[2],
            headroom: 0.0,
            opacity: 1.0,
        }
    }
}
