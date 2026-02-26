//! Filter implementations.
//!
//! Each filter is a pure data struct implementing the [`Filter`](crate::Filter) trait.
//! Filters with `COLOR_ONLY = true` can be automatically fused with adjacent
//! color-only filters for better performance.

mod blur;
mod brightness;
mod contrast;
mod grayscale;
mod hue_rotation;
mod invert;
mod saturation;
mod sepia;
mod sharpen;
mod vignette;

pub use blur::Blur;
pub use brightness::Brightness;
pub use contrast::Contrast;
pub use grayscale::Grayscale;
pub use hue_rotation::HueRotation;
pub use invert::Invert;
pub use saturation::Saturation;
pub use sepia::Sepia;
pub use sharpen::Sharpen;
pub use vignette::Vignette;
