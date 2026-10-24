//! The engine's WGSL capability floor, shared by `build.rs` and the runtime.

use naga::valid::Capabilities;

/// The naga capabilities every engine WGSL module is validated against.
///
/// This is the floor every device the engine supports provides: the engine
/// WGSL calls `quantizeToF16`, which is core WGSL but which naga gates behind
/// `SHADER_FLOAT16_IN_FLOAT32`, and the image variant's bilinear taps
/// interpolate in `f16`, which `SHADER_FLOAT16` licenses. The build script
/// validates the fixed engine modules with it and the runtime validates user
/// shaders (which are composed into those modules) with it, so a module
/// cannot pass one and fail the other.
pub const ENGINE_CAPABILITIES: Capabilities =
    Capabilities::SHADER_FLOAT16_IN_FLOAT32.union(Capabilities::SHADER_FLOAT16);
