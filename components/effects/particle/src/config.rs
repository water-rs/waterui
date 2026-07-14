//! Internal configuration types for the particle system.
//!
//! These structs are used internally and built via the flat modifier API on `ParticleSystem`.

use waterui_core::Computed;
use waterui_graphics::color::Color;

/// Emitter shape for particle spawning.
///
/// `#[non_exhaustive]` lets us add new emitter shapes (line, spline, polygon,
/// mesh surface…) without breaking downstream `match` statements.
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub enum EmitterShape {
    /// Emit from a single point.
    #[default]
    Point,
    /// Emit from a rectangle with given width and height.
    Rect {
        /// Rectangle width in normalized coordinates.
        width: f32,
        /// Rectangle height in normalized coordinates.
        height: f32,
    },
    /// Emit from a circle with given radius.
    Circle {
        /// Circle radius in normalized coordinates.
        radius: f32,
    },
}

/// Blend mode for particle rendering.
///
/// `#[non_exhaustive]` reserves room for future blend modes such as
/// multiply, screen, or premultiplied-alpha variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlendMode {
    /// Standard alpha blending.
    #[default]
    Alpha,
    /// Additive blending (for fire, glow, sparks).
    Additive,
}

/// Internal emitter configuration.
#[derive(Clone, Debug)]
pub struct EmitterConfig {
    pub position: Computed<[f32; 2]>,
    pub shape: Computed<EmitterShape>,
    pub rate: Computed<f32>,
}

impl Default for EmitterConfig {
    fn default() -> Self {
        Self {
            position: Computed::constant([0.5, 0.5]),
            shape: Computed::constant(EmitterShape::Point),
            rate: Computed::constant(100.0),
        }
    }
}

/// Particle shape for SDF rendering.
///
/// `#[non_exhaustive]` reserves room for future SDF primitives such as
/// triangles, stars, or sprites.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ParticleShape {
    /// Circular particle sprite.
    #[default]
    Circle,
    /// Rectangular particle sprite.
    Rect,
}

/// Internal particle properties configuration.
#[derive(Clone, Debug)]
pub struct ParticleProps {
    pub life: Computed<[f32; 2]>,
    pub speed: Computed<[f32; 2]>,
    pub angle: Computed<[f32; 2]>,
    pub size: Computed<[f32; 2]>,
    pub spin: Computed<[f32; 2]>,
    /// Color at start of particle life (user-provided Color, resolved later).
    pub color_start: Computed<Color>,
    /// Color at end of particle life (user-provided Color, resolved later).
    pub color_end: Computed<Color>,
    pub stretch_with_velocity: bool,
    /// Edge softness for SDF rendering 0.0 (hard) to 1.0 (soft).
    pub softness: Computed<f32>,
    pub shape: ParticleShape,
}

impl Default for ParticleProps {
    fn default() -> Self {
        Self {
            life: Computed::constant([1.0, 2.0]),
            speed: Computed::constant([0.5, 1.0]),
            angle: Computed::constant([0.0, core::f32::consts::TAU]),
            size: Computed::constant([0.01, 0.02]),
            color_start: Computed::constant(Color::srgb(255, 255, 255)),
            color_end: Computed::constant(Color::srgb(255, 255, 255).with_opacity(0.0)),
            stretch_with_velocity: false,
            softness: Computed::constant(0.5),
            shape: ParticleShape::Circle,
            spin: Computed::constant([0.0, 0.0]),
        }
    }
}

/// Internal environment configuration.
#[derive(Clone, Debug)]
pub struct EnvironmentConfig {
    pub gravity: Computed<[f32; 2]>,
    pub wind: Computed<[f32; 2]>,
    pub drag: Computed<f32>,
    pub turbulence: Computed<f32>,
}

impl Default for EnvironmentConfig {
    fn default() -> Self {
        Self {
            gravity: Computed::constant([0.0, 0.0]),
            wind: Computed::constant([0.0, 0.0]),
            drag: Computed::constant(1.0),
            turbulence: Computed::constant(0.0),
        }
    }
}

/// Internal collision configuration.
#[derive(Clone, Debug)]
pub struct CircleObstacleConfig {
    pub value: Computed<[f32; 3]>,
}

/// Internal particle-particle interaction configuration.
#[derive(Clone, Debug)]
pub struct ParticleInteractionConfig {
    pub enabled: bool,
    pub radius: Computed<f32>,
    pub strength: Computed<f32>,
}

impl Default for ParticleInteractionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            radius: Computed::constant(0.0),
            strength: Computed::constant(0.0),
        }
    }
}

/// Internal collision configuration.
#[derive(Clone, Debug)]
pub struct CollisionConfig {
    pub enabled: bool,
    /// Bounds encoded as `min_x`, `min_y`, `max_x`, `max_y` in normalized coordinates.
    pub bounds: Computed<[f32; 4]>,
    pub restitution: Computed<f32>,
    pub surface_friction: Computed<f32>,
    pub circle_obstacles: Vec<CircleObstacleConfig>,
}

impl Default for CollisionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            bounds: Computed::constant([0.0, 0.0, 1.0, 1.0]),
            restitution: Computed::constant(1.0),
            surface_friction: Computed::constant(1.0),
            circle_obstacles: Vec::new(),
        }
    }
}

/// Full internal configuration (built by modifier chain).
#[derive(Clone, Debug, Default)]
pub struct ParticleConfig {
    pub emitter: EmitterConfig,
    pub particle: ParticleProps,
    pub environment: EnvironmentConfig,
    pub collision: CollisionConfig,
    pub interaction: ParticleInteractionConfig,
    pub blend_mode: BlendMode,
}
