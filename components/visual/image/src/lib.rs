//! Image primitives and decode helpers for `WaterUI`.

extern crate alloc;

/// Image decode routing and HEIF compatibility helpers.
pub mod codec;
mod image;

use shaderloom::CompiledShader;

const IMAGE_RENDER_SHADER: CompiledShader = include!(concat!(env!("OUT_DIR"), "/image_render.rs"));

pub use codec::DecodePath;
pub use image::{Image, Interpolation, ReactiveImage, ReactiveImageHandle, image, reactive_image};
