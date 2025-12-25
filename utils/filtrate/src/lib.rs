//! GPU texture filters using wgpu.
//!
//! `filtrate` provides GPU-accelerated image filters that work with any wgpu texture.
//! It's designed to be standalone and usable outside of WaterUI - for images, video frames,
//! render targets, or any GPU texture.
//!
//! # Features
//!
//! - **Blur**: Gaussian blur with configurable radius
//! - **Brightness**: Adjust image brightness
//! - **Saturation**: Control color saturation
//! - **Contrast**: Adjust image contrast
//! - **Grayscale**: Convert to grayscale
//! - **Hue Rotation**: Rotate colors around the color wheel
//! - **Invert**: Invert all colors
//! - **Sepia**: Apply sepia tone effect
//! - **Vignette**: Add vignette effect
//! - **Sharpen**: Sharpen image details
//!
//! # Example
//!
//! ```ignore
//! use filtrate::{FilterPipeline, Filter};
//!
//! // Create pipeline with existing wgpu device/queue
//! let pipeline = FilterPipeline::new(&device, &queue);
//!
//! // Apply filters to any texture
//! pipeline.apply(
//!     &input_texture,
//!     &output_texture,
//!     &[
//!         Filter::Blur { radius: 5.0 },
//!         Filter::Brightness { amount: 0.1 },
//!         Filter::Saturation { amount: 1.2 },
//!     ],
//! );
//! ```

mod pipeline;
mod uniform;

pub use pipeline::FilterPipeline;

/// GPU filter to apply to a texture.
///
/// Each filter variant represents a different image processing effect
/// that runs entirely on the GPU via wgpu compute/render shaders.
#[derive(Debug, Clone, PartialEq)]
pub enum Filter {
    /// Gaussian blur effect.
    Blur {
        /// Blur radius in pixels (0.0 = no blur, higher = more blur)
        radius: f32,
    },

    /// Adjust brightness.
    Brightness {
        /// Brightness adjustment (-1.0 = black, 0.0 = unchanged, 1.0 = white)
        amount: f32,
    },

    /// Adjust color saturation.
    Saturation {
        /// Saturation multiplier (0.0 = grayscale, 1.0 = unchanged, >1.0 = more saturated)
        amount: f32,
    },

    /// Adjust contrast.
    Contrast {
        /// Contrast multiplier (0.0 = gray, 1.0 = unchanged, >1.0 = more contrast)
        amount: f32,
    },

    /// Convert to grayscale.
    Grayscale {
        /// Mix factor (0.0 = original, 1.0 = full grayscale)
        intensity: f32,
    },

    /// Rotate hue around the color wheel.
    HueRotation {
        /// Rotation angle in degrees (0-360)
        angle: f32,
    },

    /// Invert all colors.
    Invert,

    /// Apply sepia tone effect.
    Sepia {
        /// Sepia intensity (0.0 = original, 1.0 = full sepia)
        intensity: f32,
    },

    /// Add vignette effect (darkened corners).
    Vignette {
        /// Inner radius where vignette starts (0.0-1.0)
        radius: f32,
        /// How soft the vignette edge is (0.0-1.0)
        softness: f32,
    },

    /// Sharpen image details.
    Sharpen {
        /// Sharpening strength (0.0 = unchanged, 1.0 = normal, >1.0 = more sharp)
        amount: f32,
    },
}

impl Filter {
    /// Returns the filter type ID used internally for shader selection.
    #[allow(dead_code)]
    pub(crate) const fn type_id(&self) -> u32 {
        match self {
            Self::Blur { .. } => 0,
            Self::Brightness { .. } => 1,
            Self::Saturation { .. } => 2,
            Self::Contrast { .. } => 3,
            Self::Grayscale { .. } => 4,
            Self::HueRotation { .. } => 5,
            Self::Invert => 6,
            Self::Sepia { .. } => 7,
            Self::Vignette { .. } => 8,
            Self::Sharpen { .. } => 9,
        }
    }
}
