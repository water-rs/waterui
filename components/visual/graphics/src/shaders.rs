//! Build-time compiled shaders used by GPU renderers.

use shaderloom::CompiledShader;

/// Animated mesh-gradient shader.
pub const ANIMATED_MESH_GRADIENT: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/animated_mesh_gradient.rs"));

/// General mesh-gradient shader.
pub const MESH_GRADIENT: CompiledShader = include!(concat!(env!("OUT_DIR"), "/mesh_gradient.rs"));

/// Scene texture blit shader.
pub const BLIT: CompiledShader = include!(concat!(env!("OUT_DIR"), "/blit.rs"));

/// Built-in flowing-gradient `ShaderSurface` module.
pub const FLOWING_GRADIENT: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/flowing_gradient.rs"));
