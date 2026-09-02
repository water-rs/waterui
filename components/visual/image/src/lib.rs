//! Image primitives and decode helpers for `WaterUI`.
// Proving `Send` across `wgpu`'s generic type graph is deeper than rustc's
// default recursion limit of 128 on the workspace's nightly toolchain, which
// reports `overflow evaluating the requirement ...: Send` — a hard error under
// `-D warnings`. The bound genuinely holds; the solver just needs room to say
// so. Harmless on stable, where the limit is never reached.
#![recursion_limit = "256"]

extern crate alloc;

/// Image decode routing and HEIF compatibility helpers.
pub mod codec;
mod image;

use shaderloom::CompiledShader;

const IMAGE_RENDER_SHADER: CompiledShader = include!(concat!(env!("OUT_DIR"), "/image_render.rs"));

pub use codec::DecodePath;
pub use image::{Image, Interpolation, ReactiveImage, ReactiveImageHandle, image, reactive_image};
