//! Benchmark harness surface for the YUV plane decode.
//!
//! This module exists so the benchmark can bake the same uniform and
//! include the same decode shader the engine uses. It is not a host
//! contract.

pub use crate::render::external::{Params as YuvFrameParams, YuvLayout, yuv_frame_params};

/// `shared.wgsl` followed by `external.wgsl`, the integer-plane decode.
pub const DECODE_WGSL: &str = concat!(
    include_str!("render/shared.wgsl"),
    include_str!("render/external.wgsl"),
);
