//! Internal configuration types for the particle system.
//!
//! These structs are used internally and built via the flat modifier API on `ParticleSystem`.

use core::ops::Range;
use waterui_graphics::color::Color;

/// Emitter shape for particle spawning.
#[derive(Clone, Copy, Debug, Default)]
pub enum EmitterShape {
    /// Emit from a single point.
    #[default]
    Point,
    /// Emit from a rectangle with given width and height.
    Rect { width: f32, height: f32 },
    /// Emit from a circle with given radius.
    Circle { radius: f32 },
}

/// Blend mode for particle rendering.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BlendMode {
    /// Standard alpha blending.
    #[default]
    Alpha,
    /// Additive blending (for fire, glow, sparks).
    Additive,
}

/// Internal emitter configuration.
#[derive(Clone, Debug)]
pub(crate) struct EmitterConfig {
    pub position: [f32; 2],
    pub shape: EmitterShape,
    pub rate: f32,
    pub enabled: bool,
}

impl Default for EmitterConfig {
    fn default() -> Self {
        Self {
            position: [0.5, 0.5],
            shape: EmitterShape::Point,
            rate: 100.0,
            enabled: true,
        }
    }
}

/// Particle shape for SDF rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ParticleShape {
    #[default]
    Circle,
    Rect,
}

/// Internal particle properties configuration.
#[derive(Clone, Debug)]
pub(crate) struct ParticleProps {
    pub life: Range<f32>,
    pub speed: Range<f32>,
    pub angle: Range<f32>,
    pub size: Range<f32>,
    pub spin: Range<f32>, // Rotation speed in rad/s
    /// Color at start of particle life (user-provided Color, resolved later).
    pub color_start: Color,
    /// Color at end of particle life (user-provided Color, resolved later).
    pub color_end: Color,
    pub stretch_with_velocity: bool,
    /// Edge softness for SDF rendering 0.0 (hard) to 1.0 (soft).
    pub softness: f32,
    pub shape: ParticleShape,
}

impl Default for ParticleProps {
    fn default() -> Self {
        Self {
            life: 1.0..2.0,
            speed: 0.5..1.0,
            angle: 0.0..core::f32::consts::TAU,
            size: 0.01..0.02,
            color_start: Color::srgb(255, 255, 255),
            color_end: Color::srgb(255, 255, 255).with_opacity(0.0),
            stretch_with_velocity: false,
            softness: 0.5,
            shape: ParticleShape::Circle,
            spin: 0.0..0.0,
        }
    }
}

/// Internal environment configuration.
#[derive(Clone, Debug)]
pub(crate) struct EnvironmentConfig {
    pub gravity: [f32; 2],
    pub wind: [f32; 2],
    pub drag: f32,
    pub turbulence: f32,
}

impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            gravity: [0.0, 0.0],
            wind: [0.0, 0.0],
            drag: 1.0,
            turbulence: 0.0,
        }
    }
}

/// Internal collision configuration.
#[derive(Clone, Debug)]
pub(crate) struct CircleObstacleConfig {
    pub center: [f32; 2],
    pub radius: f32,
}

/// Internal collision configuration.
#[derive(Clone, Debug)]
pub(crate) struct CollisionConfig {
    pub enabled: bool,
    /// Bounds encoded as min_x, min_y, max_x, max_y in normalized coordinates.
    pub bounds: [f32; 4],
    pub restitution: f32,
    pub surface_friction: f32,
    pub circle_obstacles: Vec<CircleObstacleConfig>,
}

impl Default for CollisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bounds: [0.0, 0.0, 1.0, 1.0],
            restitution: 1.0,
            surface_friction: 1.0,
            circle_obstacles: Vec::new(),
        }
    }
}

/// Full internal configuration (built by modifier chain).
#[derive(Clone, Debug, Default)]
pub(crate) struct ParticleConfig {
    pub emitter: EmitterConfig,
    pub particle: ParticleProps,
    pub environment: EnvironmentConfig,
    pub collision: CollisionConfig,
    pub blend_mode: BlendMode,
}
