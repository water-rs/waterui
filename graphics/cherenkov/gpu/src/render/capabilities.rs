//! The engine's WGSL capability floor, shared by `build.rs` and the runtime.

use naga::valid::Capabilities;

/// The naga capabilities every engine WGSL module is validated against.
///
/// This is the floor every device the engine supports provides: the engine
/// WGSL uses native f16 blend intermediates and `quantizeToF16`. The build
/// script validates fixed modules and the runtime validates user shaders (composed
/// into those modules) with it, so a module cannot pass one and fail the
/// other.
pub const ENGINE_CAPABILITIES: Capabilities =
    Capabilities::SHADER_FLOAT16_IN_FLOAT32.union(Capabilities::SHADER_FLOAT16);
