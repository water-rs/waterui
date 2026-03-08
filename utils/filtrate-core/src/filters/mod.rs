//! Filter implementations.
//!
//! Each filter is a pure data struct implementing the [`Filter`](crate::Filter) trait.
//! Filters with `COLOR_ONLY = true` can be automatically fused with adjacent
//! color-only filters for better performance.

mod bloom;
mod blur;
mod brightness;
mod bump_distortion;
mod color_matrix;
mod contrast;
mod crystallize;
mod dot_halftone;
mod edge_work;
mod exposure;
mod gamma;
mod gaussian_blur;
mod gloom;
mod grayscale;
mod highlights_shadows;
mod hue_rotation;
mod invert;
mod kaleidoscope;
mod line_halftone;
mod mirror_tile;
mod motion_blur;
mod perspective_correction;
mod perspective_transform;
mod pixellate;
mod pinch_distortion;
mod saturation;
mod sepia;
mod sharpen;
mod temperature_tint;
mod twirl_distortion;
mod unsharp_mask;
mod vibrance;
mod vignette;
mod vortex_distortion;
mod white_point;
mod zoom_blur;

pub use bloom::Bloom;
pub use blur::Blur;
pub use brightness::Brightness;
pub use bump_distortion::BumpDistortion;
pub use color_matrix::ColorMatrix;
pub use contrast::Contrast;
pub use crystallize::Crystallize;
pub use dot_halftone::DotHalftone;
pub use edge_work::EdgeWork;
pub use exposure::Exposure;
pub use gamma::Gamma;
pub use gaussian_blur::GaussianBlur;
pub use gloom::Gloom;
pub use grayscale::Grayscale;
pub use highlights_shadows::HighlightsShadows;
pub use hue_rotation::HueRotation;
pub use invert::Invert;
pub use kaleidoscope::Kaleidoscope;
pub use line_halftone::LineHalftone;
pub use mirror_tile::MirrorTile;
pub use motion_blur::MotionBlur;
pub use perspective_correction::PerspectiveCorrection;
pub use perspective_transform::PerspectiveTransform;
pub use pixellate::Pixellate;
pub use pinch_distortion::PinchDistortion;
pub use saturation::Saturation;
pub use sepia::Sepia;
pub use sharpen::Sharpen;
pub use temperature_tint::TemperatureTint;
pub use twirl_distortion::TwirlDistortion;
pub use unsharp_mask::UnsharpMask;
pub use vibrance::Vibrance;
pub use vignette::Vignette;
pub use vortex_distortion::VortexDistortion;
pub use white_point::WhitePoint;
pub use zoom_blur::ZoomBlur;
