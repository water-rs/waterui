//! Build-time compiled shaders for the particle system.

use shaderloom::CompiledShader;

pub const COMPUTE_SHADER: CompiledShader =
    include!(concat!(env!("OUT_DIR"), "/particle_compute.rs"));
pub const RENDER_SHADER: CompiledShader = include!(concat!(env!("OUT_DIR"), "/particle_render.rs"));
