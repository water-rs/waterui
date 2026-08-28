//! GPU buffer structs with correct alignment for wgpu.
//!
//! Uses encase for automatic WGSL-compatible alignment.

use encase::ShaderType;
use waterui_graphics::shader_types::{ShaderVec2, ShaderVec4};

/// GPU representation of a single particle.
/// Uses explicit padding so the storage layout also works as an instanced vertex buffer.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, ShaderType, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParticle {
    /// Position in normalized coordinates [0, 1].
    pub pos: ShaderVec2,
    /// Velocity.
    pub vel: ShaderVec2,
    /// Current life remaining.
    pub life: f32,
    /// Maximum life (for interpolation ratio).
    pub max_life: f32,
    /// Particle size.
    pub size: f32,
    /// Current rotation (radians).
    pub rotation: f32,
    /// Rotation speed (radians/sec).
    pub rot_speed: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
    /// Color in Linear sRGB.
    pub color: ShaderVec4,
}

/// GPU uniforms for compute and render shaders.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct InteractionUniforms {
    /// Whether particle-particle interaction is active.
    pub enabled: u32,
    /// Uniform grid width in cells.
    pub grid_width: u32,
    /// Uniform grid height in cells.
    pub grid_height: u32,
    /// Additional interaction radius beyond particle size.
    pub radius: f32,
    /// Velocity response strength.
    pub strength: f32,
}

impl InteractionUniforms {
    #[must_use]
    pub fn new(
        enabled: bool,
        grid_width: u32,
        grid_height: u32,
        radius: f32,
        strength: f32,
    ) -> Self {
        Self {
            enabled: u32::from(enabled),
            grid_width,
            grid_height,
            radius,
            strength,
        }
    }
}

/// GPU uniforms for compute and render shaders.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct CollisionUniforms {
    /// Whether collision response is active.
    pub enabled: u32,
    /// Velocity preserved along the collision normal.
    pub restitution: f32,
    /// Velocity preserved tangent to the collision surface.
    pub surface_friction: f32,
    /// Number of circular obstacle colliders in the storage buffer.
    pub circle_obstacle_count: u32,
    /// Collision bounds encoded as `min_x`, `min_y`, `max_x`, `max_y`.
    pub bounds: ShaderVec4,
}

/// GPU representation of a circular obstacle collider.
#[derive(Clone, Copy, Debug, Default, ShaderType)]
pub struct GpuCircleObstacle {
    pub center: ShaderVec2,
    pub radius: f32,
}

impl CollisionUniforms {
    #[must_use]
    pub fn new(
        enabled: bool,
        restitution: f32,
        surface_friction: f32,
        bounds: ShaderVec4,
        circle_obstacle_count: u32,
    ) -> Self {
        Self {
            enabled: u32::from(enabled),
            restitution,
            surface_friction,
            circle_obstacle_count,
            bounds,
        }
    }
}

impl GpuCircleObstacle {
    #[must_use]
    pub const fn new(center: ShaderVec2, radius: f32) -> Self {
        Self { center, radius }
    }
}

/// GPU uniforms for compute and render shaders.
/// Uses encase for automatic WGSL-compatible alignment.
#[derive(Clone, Copy, Debug, ShaderType)]
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
    pub gravity: ShaderVec2,
    /// Wind vector.
    pub wind: ShaderVec2,
    /// Emitter position.
    pub emitter_pos: ShaderVec2,
    /// Emitter size.
    /// `Rect`: `(width, height)`, `Circle`: `(radius, -1.0)`, `Point`: `(0.0, 0.0)`.
    pub emitter_size: ShaderVec2,
    /// Emission rate (particles per second).
    pub emit_rate: f32,
    /// Turbulence factor.
    pub turbulence: f32,
    /// Velocity damping factor (1.0 = no damping, 0.0 = stop immediately).
    pub drag: f32,
    /// Velocity stretch factor (0.0 = disabled, >0.0 = magnitude scale).
    pub stretch_factor: f32,
    /// Edge softness (0.0=hard, 1.0=soft).
    pub softness: f32,
    /// Particle-particle interaction configuration.
    pub interaction: InteractionUniforms,
    /// Collision configuration.
    pub collision: CollisionUniforms,
    /// Life range (min, max).
    pub life_range: ShaderVec2,
    /// Speed range (min, max).
    pub speed_range: ShaderVec2,
    /// Angle range (min, max) in radians.
    pub angle_range: ShaderVec2,
    /// Size range (min, max).
    pub size_range: ShaderVec2,
    /// Spin speed range (min, max) in radians/sec.
    pub spin_range: ShaderVec2,
    /// Start color.
    pub color_start: ShaderVec4,
    /// End color.
    pub color_end: ShaderVec4,
    /// Particle shape (0=Circle, 1=Rect).
    pub shape: u32,
    /// Viewport width in pixels.
    pub viewport_width: u32,
    /// Viewport height in pixels.
    pub viewport_height: u32,
}

impl Default for Uniforms {
    fn default() -> Self {
        Self {
            time: 0.0,
            dt: 1.0 / 60.0,
            seed: 0,
            max_particles: 1000,
            gravity: ShaderVec2::default(),
            wind: ShaderVec2::default(),
            emitter_pos: ShaderVec2::new(0.5, 0.5),
            emitter_size: ShaderVec2::default(),
            emit_rate: 100.0,
            turbulence: 0.0,
            drag: 1.0,
            stretch_factor: 0.0,
            softness: 0.5,
            interaction: InteractionUniforms::default(),
            collision: CollisionUniforms::default(),
            life_range: ShaderVec2::new(1.0, 1.0),
            speed_range: ShaderVec2::new(1.0, 1.0),
            angle_range: ShaderVec2::default(),
            size_range: ShaderVec2::new(0.01, 0.01),
            spin_range: ShaderVec2::default(),
            color_start: ShaderVec4::ONE,
            color_end: ShaderVec4::ONE,
            shape: 0,
            viewport_width: 0,
            viewport_height: 0,
        }
    }
}
