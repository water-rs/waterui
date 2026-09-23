//! `ParticleSystem` View with ergonomic API.

use crate::{
    EmitterShape,
    config::{BlendMode, CircleObstacleConfig, ParticleConfig},
    renderer::{ParticleRenderer, ResolvedParticleConfig},
};
use core::{num::NonZeroU32, ops::Range};
use waterui_core::{Environment, IntoSignal, IntoSignalF32, Signal, View};
use waterui_graphics::{
    GpuSurface, OffscreenRenderConfig, OffscreenRenderError, OffscreenRenderOutput,
    OffscreenRenderOutputHdr, color::Color,
};

/// High-performance GPU particle system.
///
/// Use the flat modifier-chain API to configure the particle system,
/// then use it as a View in your UI hierarchy.
#[derive(Clone, Debug)]
pub struct ParticleSystem {
    max_particles: u32,
    pub(crate) config: ParticleConfig,
}

fn snapshot_f32(signal: impl IntoSignalF32 + 'static) -> f32 {
    signal.into_signal_f32().get()
}

fn snapshot_color(signal: impl IntoSignal<Color> + 'static) -> Color {
    signal.into_signal().get()
}

fn snapshot_range_f32(
    start: impl IntoSignalF32 + 'static,
    end: impl IntoSignalF32 + 'static,
) -> Range<f32> {
    snapshot_f32(start)..snapshot_f32(end)
}

impl ParticleSystem {
    /// Create a new particle system with a maximum particle count.
    #[must_use]
    pub fn new(max_particles: u32) -> Self {
        Self {
            max_particles,
            config: {
                let mut c = ParticleConfig::default();
                c.particle.spin = 0.0..0.0;
                c
            },
        }
    }

    /// Set emission rate (particles per second).
    #[must_use]
    pub fn rate(mut self, rate: impl IntoSignalF32 + 'static) -> Self {
        self.config.emitter.rate = snapshot_f32(rate);
        self
    }

    /// Set emitter as a single point.
    #[must_use]
    pub const fn emit_from_point(mut self) -> Self {
        self.config.emitter.shape = EmitterShape::Point;
        self
    }

    /// Set emitter as a rectangle.
    #[must_use]
    pub fn emit_from_rect(
        mut self,
        width: impl IntoSignalF32 + 'static,
        height: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.emitter.shape = EmitterShape::Rect {
            width: snapshot_f32(width),
            height: snapshot_f32(height),
        };
        self
    }

    /// Set emitter position (normalized coordinates 0.0-1.0).
    #[must_use]
    pub fn at(mut self, x: impl IntoSignalF32 + 'static, y: impl IntoSignalF32 + 'static) -> Self {
        self.config.emitter.position = [snapshot_f32(x), snapshot_f32(y)];
        self
    }

    /// Set particle lifespan range.
    #[must_use]
    pub fn life(
        mut self,
        start: impl IntoSignalF32 + 'static,
        end: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.particle.life = snapshot_range_f32(start, end);
        self
    }

    /// Set particle speed range.
    #[must_use]
    pub fn speed(
        mut self,
        start: impl IntoSignalF32 + 'static,
        end: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.particle.speed = snapshot_range_f32(start, end);
        self
    }

    /// Set particle emission angle range (in radians).
    #[must_use]
    pub fn angle(
        mut self,
        start: impl IntoSignalF32 + 'static,
        end: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.particle.angle = snapshot_range_f32(start, end);
        self
    }

    /// Set particle size range.
    #[must_use]
    pub fn size(
        mut self,
        start: impl IntoSignalF32 + 'static,
        end: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.particle.size = snapshot_range_f32(start, end);
        self
    }

    /// Set particle start and end colors.
    #[must_use]
    pub fn color(
        mut self,
        start: impl IntoSignal<Color> + 'static,
        end: impl IntoSignal<Color> + 'static,
    ) -> Self {
        self.config.particle.color_start = snapshot_color(start);
        self.config.particle.color_end = snapshot_color(end);
        self
    }

    /// Enable motion blur (stretch particles based on velocity).
    #[must_use]
    pub const fn stretch_with_velocity(mut self) -> Self {
        self.config.particle.stretch_with_velocity = true;
        self
    }

    /// Set gravity vector.
    #[must_use]
    pub fn gravity(
        mut self,
        x: impl IntoSignalF32 + 'static,
        y: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.environment.gravity = [snapshot_f32(x), snapshot_f32(y)];
        self
    }

    /// Set wind vector.
    #[must_use]
    pub fn wind(
        mut self,
        x: impl IntoSignalF32 + 'static,
        y: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.environment.wind = [snapshot_f32(x), snapshot_f32(y)];
        self
    }

    /// Keep particles inside the normalized viewport `[0, 0]..[1, 1]`.
    #[must_use]
    pub const fn collide_with_viewport(mut self) -> Self {
        self.config.collision.enabled = true;
        self.config.collision.bounds = [0.0, 0.0, 1.0, 1.0];
        self
    }

    /// Keep particles inside a normalized rectangle.
    #[must_use]
    pub fn collide_with_rect(
        mut self,
        x: impl IntoSignalF32 + 'static,
        y: impl IntoSignalF32 + 'static,
        width: impl IntoSignalF32 + 'static,
        height: impl IntoSignalF32 + 'static,
    ) -> Self {
        let x = snapshot_f32(x);
        let y = snapshot_f32(y);
        let width = snapshot_f32(width);
        let height = snapshot_f32(height);
        self.config.collision.enabled = true;
        self.config.collision.bounds = [x, y, x + width, y + height];
        self
    }

    /// Add a circular obstacle collider in normalized coordinates.
    #[must_use]
    pub fn collide_with_circle_obstacle(
        mut self,
        x: impl IntoSignalF32 + 'static,
        y: impl IntoSignalF32 + 'static,
        radius: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config
            .collision
            .circle_obstacles
            .push(CircleObstacleConfig {
                center: [snapshot_f32(x), snapshot_f32(y)],
                radius: snapshot_f32(radius),
            });
        self
    }

    /// Set the fraction of normal velocity preserved after a collision.
    #[must_use]
    pub fn bounce(mut self, restitution: impl IntoSignalF32 + 'static) -> Self {
        self.config.collision.restitution = snapshot_f32(restitution);
        self
    }

    /// Set the fraction of tangential velocity preserved after a collision.
    #[must_use]
    pub fn surface_friction(mut self, value: impl IntoSignalF32 + 'static) -> Self {
        self.config.collision.surface_friction = snapshot_f32(value);
        self
    }

    /// Enable pure-GPU particle-particle interaction using a neighbor grid.
    #[must_use]
    pub fn collide_with_particles(
        mut self,
        radius: impl IntoSignalF32 + 'static,
        strength: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.interaction.enabled = true;
        self.config.interaction.radius = snapshot_f32(radius);
        self.config.interaction.strength = snapshot_f32(strength);
        self
    }

    /// Set additive blending mode.
    #[must_use]
    pub const fn additive(mut self) -> Self {
        self.config.blend_mode = BlendMode::Additive;
        self
    }

    /// Set edge softness (0.0=hard, 1.0=soft).
    #[must_use]
    pub fn softness(mut self, value: impl IntoSignalF32 + 'static) -> Self {
        self.config.particle.softness = snapshot_f32(value);
        self
    }

    /// Set turbulence strength.
    #[must_use]
    pub fn turbulence(mut self, value: impl IntoSignalF32 + 'static) -> Self {
        self.config.environment.turbulence = snapshot_f32(value);
        self
    }

    /// Set velocity damping factor normalized to a 60 FPS baseline.
    ///
    /// `1.0` keeps velocity unchanged, lower values damp motion over time
    /// without changing behavior across frame rates.
    #[must_use]
    pub fn drag(mut self, value: impl IntoSignalF32 + 'static) -> Self {
        self.config.environment.drag = snapshot_f32(value);
        self
    }

    /// Set emitter as a disk with the given radius.
    #[must_use]
    pub fn emit_from_circle(mut self, radius: impl IntoSignalF32 + 'static) -> Self {
        self.config.emitter.shape = EmitterShape::Circle {
            radius: snapshot_f32(radius),
        };
        self
    }

    /// Set particle shape.
    #[must_use]
    pub const fn shape(mut self, shape: crate::config::ParticleShape) -> Self {
        self.config.particle.shape = shape;
        self
    }

    /// Set initial particle spin speed range (radians/sec).
    #[must_use]
    pub fn spin(
        mut self,
        start: impl IntoSignalF32 + 'static,
        end: impl IntoSignalF32 + 'static,
    ) -> Self {
        self.config.particle.spin = snapshot_range_f32(start, end);
        self
    }

    fn resolved_config(self, env: &Environment) -> ResolvedParticleConfig {
        let color_start = self.config.particle.color_start.resolve(env).get();
        let color_end = self.config.particle.color_end.resolve(env).get();

        ResolvedParticleConfig {
            max_particles: self.max_particles,
            emitter_pos: self.config.emitter.position,
            emitter_shape: self.config.emitter.shape,
            emit_rate: self.config.emitter.rate,
            gravity: self.config.environment.gravity,
            wind: self.config.environment.wind,
            turbulence: self.config.environment.turbulence,
            drag: self.config.environment.drag,
            collision_enabled: self.config.collision.enabled,
            collision_bounds: self.config.collision.bounds,
            collision_restitution: self.config.collision.restitution,
            collision_surface_friction: self.config.collision.surface_friction,
            collision_circle_obstacles: self
                .config
                .collision
                .circle_obstacles
                .into_iter()
                .map(|obstacle| [obstacle.center[0], obstacle.center[1], obstacle.radius])
                .collect(),
            interaction_enabled: self.config.interaction.enabled,
            interaction_radius: self.config.interaction.radius,
            interaction_strength: self.config.interaction.strength,
            life_range: [
                self.config.particle.life.start,
                self.config.particle.life.end,
            ],
            speed_range: [
                self.config.particle.speed.start,
                self.config.particle.speed.end,
            ],
            angle_range: [
                self.config.particle.angle.start,
                self.config.particle.angle.end,
            ],
            size_range: [
                self.config.particle.size.start,
                self.config.particle.size.end,
            ],
            spin_range: [
                self.config.particle.spin.start,
                self.config.particle.spin.end,
            ],
            color_start,
            color_end,
            stretch_with_velocity: self.config.particle.stretch_with_velocity,
            blend_mode: self.config.blend_mode,
            softness: self.config.particle.softness,
            shape: self.config.particle.shape,
        }
    }

    /// Render the particle system to an offscreen RGBA8 target via `GpuSurface`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying GPU offscreen render fails.
    pub fn render_offscreen(
        self,
        config: OffscreenRenderConfig,
        env: &mut Environment,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        self.render_offscreen_frames(config, env, NonZeroU32::MIN)
    }

    /// Render the particle system to an offscreen RGBA8 target for `frame_count` frames via `GpuSurface`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying GPU offscreen render fails.
    pub fn render_offscreen_frames(
        self,
        config: OffscreenRenderConfig,
        env: &mut Environment,
        frame_count: NonZeroU32,
    ) -> Result<OffscreenRenderOutput, OffscreenRenderError> {
        let resolved = self.resolved_config(env);
        GpuSurface::new(ParticleRenderer::new(resolved)).render_offscreen_frames(
            config,
            env,
            frame_count,
        )
    }

    /// Render the particle system to an HDR offscreen target via `GpuSurface`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying GPU offscreen render fails.
    pub fn render_offscreen_hdr(
        self,
        config: OffscreenRenderConfig,
        env: &mut Environment,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        self.render_offscreen_hdr_frames(config, env, NonZeroU32::MIN)
    }

    /// Render the particle system to an HDR offscreen target for `frame_count` frames via `GpuSurface`.
    ///
    /// # Errors
    ///
    /// Returns an error when the underlying GPU offscreen render fails.
    pub fn render_offscreen_hdr_frames(
        self,
        config: OffscreenRenderConfig,
        env: &mut Environment,
        frame_count: NonZeroU32,
    ) -> Result<OffscreenRenderOutputHdr, OffscreenRenderError> {
        let resolved = self.resolved_config(env);
        GpuSurface::new(ParticleRenderer::new(resolved)).render_offscreen_hdr_frames(
            config,
            env,
            frame_count,
        )
    }
}

impl View for ParticleSystem {
    fn body(self, env: &Environment) -> impl View {
        GpuSurface::new(ParticleRenderer::new(self.resolved_config(env)))
    }
}

/// Convenience constructor for [`ParticleSystem`] with `max_particles` capacity.
#[must_use]
pub fn particles(max_particles: u32) -> ParticleSystem {
    ParticleSystem::new(max_particles)
}
