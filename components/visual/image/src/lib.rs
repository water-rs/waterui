//! Image primitives and decode helpers for `WaterUI`.

extern crate alloc;

/// Image decode routing and HEIF compatibility helpers.
pub mod codec;
mod image;

pub use codec::DecodePath;
pub use image::{Image, Interpolation, image};
