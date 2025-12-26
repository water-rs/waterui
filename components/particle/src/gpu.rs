//! GPU buffer structs with correct alignment for wgpu.

use bytemuck::{Pod, Zeroable};

/// GPU representation of a single particle.
/// Must be 16-byte aligned for storage buffers.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GpuParticle {
    /// Position in normalized coordinates [0, 1].
    pub pos: [f32; 2],
    /// Velocity.
    pub vel: [f32; 2],
    /// Current life remaining.
    pub life: f32,
    /// Maximum life (for interpolation ratio).
    pub max_life: f32,
    /// Particle size.
    pub size: f32,
    /// Padding for 16-byte alignment.
    pub _pad: f32,
    /// Color in Linear sRGB.
    pub color: [f32; 4],
}

impl Default for GpuParticle {
    fn default() -> Self {
        Self {
            pos: [0.0, 0.0],
            vel: [0.0, 0.0],
            life: 0.0,
            max_life: 1.0,
            size: 0.01,
            _pad: 0.0,
            color: [1.0, 1.0, 1.0, 1.0],
        }
    }
}

/// GPU uniforms for compute and render shaders.
/// 64 bytes, 16-byte aligned.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Uniforms {
    /// Elapsed time in seconds.
    pub time: f32,
    /// Delta time since last frame.
    pub dt: f32,
    /// Random seed for this frame.
    pub seed: u32,
    /// Maximum particle count.
    pub max_particles: u32,
    /// Gravity vector.
    pub gravity: [f32; 2],
    /// Wind vector.
    pub wind: [f32; 2],
    /// Emitter position.
    pub emitter_pos: [f32; 2],
    /// Emitter size (width, height for rect; radius, 0 for circle).
    pub emitter_size: [f32; 2],
    /// Emission rate (particles per second).
    pub emit_rate: f32,
    /// Turbulence factor.
    pub turbulence: f32,
    /// Velocity stretch factor (0.0 = disabled, >0.0 = magnitude scale).
    pub stretch_factor: f32,
    /// Edge softness (0.0=hard, 1.0=soft).
    pub softness: f32,

    /// Life range (min, max).
    pub life_range: [f32; 2],
    /// Speed range (min, max).
    pub speed_range: [f32; 2],
    
    /// Angle range (min, max) in radians.
    pub angle_range: [f32; 2],
    /// Size range (min, max).
    pub size_range: [f32; 2],

    /// Start color.
    pub color_start: [f32; 4],
    /// End color.
    pub color_end: [f32; 4],

    /// Particle shape (0=Circle, 1=Rect).
    pub shape: u32,
    /// Padding.
    pub _pad: [u32; 3],
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            time: 0.0,
            dt: 1.0 / 60.0,
            seed: 0,
            max_particles: 1000,
            gravity: [0.0, 0.0],
            wind: [0.0, 0.0],
            emitter_pos: [0.5, 0.5],
            emitter_size: [0.0, 0.0],
            emit_rate: 100.0,
            turbulence: 0.0,
            stretch_factor: 0.0,
            softness: 0.5,
            life_range: [1.0, 1.0],
            speed_range: [1.0, 1.0],
            angle_range: [0.0, 0.0],
            size_range: [0.01, 0.01],
            color_start: [1.0, 1.0, 1.0, 1.0],
            color_end: [1.0, 1.0, 1.0, 1.0],
        }
    }
}
