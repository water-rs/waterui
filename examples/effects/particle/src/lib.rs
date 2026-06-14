//! Particle Example - Demonstrates High-Performance GPU Particles
//!
//! This example showcases the `waterui-particle` crate with various effects.

use core::f32::consts::PI;
use waterui::app::App;
use waterui::color::Srgb;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::{Binding, binding};
use waterui_particle::{ParticleShape, ParticleSystem};

// --- Demos ---

fn title_label(color: impl Into<Color>) -> impl View {
    text("GPU Particle System")
        .size(24.0)
        .bold()
        .foreground(color)
}

pub fn rain_system() -> ParticleSystem {
    ParticleSystem::new(10000)
        .emit_from_rect(1.5, 0.1)
        .at(0.5, -0.05)
        .rate(2500.0)
        .life(0.6, 0.8) // Faster life
        .speed(2.5, 4.5) // Much faster
        .angle(PI * 0.49, PI * 0.51) // More vertical
        // Very thin streaks
        .size(0.0008, 0.0015)
        .color(
            Color::from(Srgb::new(0.8, 0.9, 1.0)).with_opacity(0.4),
            Color::from(Srgb::new(0.85, 0.95, 1.0)).with_opacity(0.0),
        )
        .gravity(0.0, 5.0) // Higher gravity
        .wind(0.05, 0.0) // Subtle wind
        .stretch_with_velocity()
        .softness(0.4) // Slightly softer to antialias thin lines
}

pub fn rain() -> impl View {
    rain_system()
}

fn snow() -> impl View {
    ParticleSystem::new(5000)
        // Wide emitter
        .emit_from_rect(1.5, 0.1)
        .at(0.5, -0.05)
        .rate(400.0) // More flakes
        // Long life, slow fall
        .life(4.0, 7.0)
        .speed(0.1, 0.4)
        // Slight downward angle variation
        .angle(PI * 0.4, PI * 0.6)
        // High size variation for depth
        .size(0.0015, 0.006)
        .color(
            Color::from(Srgb::WHITE).with_opacity(0.8),
            Color::from(Srgb::WHITE).with_opacity(0.0),
        )
        // Low gravity, high turbulence for flutter
        .gravity(0.0, 0.05)
        .wind(0.08, 0.0) // More wind sway
        .softness(0.5) // Sharper, less "glowing orb" look
}

fn fog() -> impl View {
    ParticleSystem::new(2000)
        // Emit from bottom
        .emit_from_rect(1.5, 0.2)
        .at(0.5, 1.1)
        .rate(50.0) // Lower rate, larger particles
        // Very slow rise
        .life(8.0, 12.0)
        .speed(0.02, 0.08)
        .angle(PI * 1.4, PI * 1.6) // Upwards
        // Large generic blobs
        .size(0.1, 0.25)
        .color(
            Color::from(Srgb::new(0.8, 0.85, 0.8)).with_opacity(0.1),
            Color::from(Srgb::new(0.8, 0.85, 0.8)).with_opacity(0.0),
        )
        .gravity(0.0, -0.01)
        .wind(0.02, 0.0)
        .softness(1.0) // Maximum softness for cloud/fog
}

pub fn flame_system() -> ParticleSystem {
    ParticleSystem::new(3000)
        .emit_from_rect(0.05, 0.0) // Small source
        .at(0.5, 0.8)
        .rate(1200.0)
        .life(0.4, 0.8)
        .speed(0.5, 1.2)
        .angle(PI * 1.4, PI * 1.6) // Up
        .size(0.03, 0.06)
        .color(
            Color::from(Srgb::new(1.0, 0.7, 0.1)).with_opacity(0.6), // Orange
            Color::from(Srgb::new(1.0, 0.1, 0.05)).with_opacity(0.0), // Red fade
        )
        .gravity(0.0, -1.0) // Rise
        .additive()
        .softness(0.6) // Soft plasma look
}

pub fn flame() -> impl View {
    flame_system()
}

fn firework() -> impl View {
    // Fountain style
    ParticleSystem::new(5000)
        .emit_from_point()
        .at(0.5, 0.8)
        .rate(2000.0)
        .life(1.0, 1.5)
        .speed(1.8, 2.8) // Fast launch
        .angle(PI * 1.35, PI * 1.65) // Cone up
        .size(0.006, 0.012)
        .color(
            Color::from(Srgb::new(1.0, 0.9, 0.6)).with_opacity(0.9), // Gold bright
            Color::from(Srgb::new(1.0, 0.4, 0.1)).with_opacity(0.0), // Orange fade
        )
        .gravity(0.0, 2.5) // Fall back down
        .stretch_with_velocity()
        .additive()
        .softness(0.2) // Sharp sparks
}

// Manual unroll for stability
fn confetti_view() -> impl View {
    // Larger since they're thin rectangles now
    let (size_start, size_end) = (0.015_f32, 0.025_f32);
    let shape = ParticleShape::Rect;
    // Random tumbling speed (rad/s)
    let (spin_start, spin_end) = (-8.0_f32, 8.0_f32);

    zstack((
        // Pink
        ParticleSystem::new(600)
            .emit_from_rect(1.5, 0.1)
            .at(0.5, -0.1)
            .rate(30.0)
            .life(5.0, 8.0)
            .speed(0.08, 0.25)
            .angle(PI * 0.4, PI * 0.6)
            .size(size_start, size_end)
            .gravity(0.0, 0.06)
            .turbulence(0.3)
            .color(
                Color::from(Srgb::new(1.0, 0.5, 0.7)),
                Color::from(Srgb::new(1.0, 0.5, 0.7)).with_opacity(0.0),
            )
            .shape(shape)
            .spin(spin_start, spin_end)
            .softness(0.4),
        // Mint Green
        ParticleSystem::new(600)
            .emit_from_rect(1.5, 0.1)
            .at(0.5, -0.1)
            .rate(30.0)
            .life(5.0, 8.0)
            .speed(0.08, 0.25)
            .angle(PI * 0.4, PI * 0.6)
            .size(size_start, size_end)
            .gravity(0.0, 0.06)
            .turbulence(0.35)
            .color(
                Color::from(Srgb::new(0.5, 0.9, 0.7)),
                Color::from(Srgb::new(0.5, 0.9, 0.7)).with_opacity(0.0),
            )
            .shape(shape)
            .spin(spin_start, spin_end)
            .softness(0.4),
        // Sky Blue
        ParticleSystem::new(600)
            .emit_from_rect(1.5, 0.1)
            .at(0.5, -0.1)
            .rate(30.0)
            .life(5.0, 8.0)
            .speed(0.08, 0.25)
            .angle(PI * 0.4, PI * 0.6)
            .size(size_start, size_end)
            .gravity(0.0, 0.06)
            .turbulence(0.4)
            .color(
                Srgb::new(0.5, 0.8, 1.0),
                Srgb::new(0.5, 0.8, 1.0).with_opacity(0.0),
            )
            .shape(shape)
            .spin(spin_start, spin_end)
            .softness(0.4),
        // Lavender
        ParticleSystem::new(600)
            .emit_from_rect(1.5, 0.1)
            .at(0.5, -0.1)
            .rate(30.0)
            .life(5.0, 8.0)
            .speed(0.08, 0.25)
            .angle(PI * 0.4, PI * 0.6)
            .size(size_start, size_end)
            .gravity(0.0, 0.06)
            .turbulence(0.45)
            .color(
                Srgb::new(0.8, 0.6, 1.0),
                Srgb::new(0.8, 0.6, 1.0).with_opacity(0.0),
            )
            .shape(shape)
            .spin(spin_start, spin_end)
            .softness(0.4),
        // Yellow
        ParticleSystem::new(600)
            .emit_from_rect(1.5, 0.1)
            .at(0.5, -0.1)
            .rate(30.0)
            .life(5.0, 8.0)
            .speed(0.08, 0.25)
            .angle(PI * 0.4, PI * 0.6)
            .size(size_start, size_end)
            .gravity(0.0, 0.06)
            .turbulence(0.5)
            .color(
                Srgb::new(1.0, 0.9, 0.4),
                Srgb::new(1.0, 0.9, 0.4).with_opacity(0.0),
            )
            .shape(shape)
            .spin(spin_start, spin_end)
            .softness(0.4),
    ))
}

pub fn explosion_system() -> ParticleSystem {
    // "Shatter" / "Explosion" - Square debris
    ParticleSystem::new(20000)
        .emit_from_circle(0.05)
        .at(0.5, 0.5)
        .rate(8000.0) // High density
        .life(0.8, 1.5)
        .speed(0.5, 3.0) // Varied speed (chunks flying out)
        .angle(0.0, PI * 2.0)
        .size(0.003, 0.008) // Tiny blocks
        .color(
            Srgb::new(1.0, 0.5, 0.0).with_opacity(1.0), // Orange
            Srgb::new(0.2, 0.2, 0.2).with_opacity(1.0), // Fade to dark grey solid
        )
        .gravity(0.0, 3.0) // Heavy pieces fall
        .shape(ParticleShape::Rect) // Square blocks
        .softness(0.0) // Hard edges
}

pub fn explosion() -> impl View {
    explosion_system()
}

fn bounce_box() -> impl View {
    ParticleSystem::new(6000)
        .emit_from_circle(0.02)
        .at(0.5, 0.18)
        .rate(420.0)
        .life(4.0, 6.0)
        .speed(0.5, 1.4)
        .angle(0.0, PI * 2.0)
        .size(0.008, 0.018)
        .color(
            Srgb::new(0.4, 0.85, 1.0).with_opacity(0.9),
            Srgb::new(0.1, 0.45, 1.0).with_opacity(0.0),
        )
        .gravity(0.0, 1.6)
        .turbulence(0.25)
        .collide_with_particles(0.01, 16.0)
        .collide_with_rect(0.08, 0.08, 0.84, 0.84)
        .collide_with_circle_obstacle(0.38, 0.34, 0.06)
        .collide_with_circle_obstacle(0.5, 0.36, 0.08)
        .collide_with_circle_obstacle(0.62, 0.34, 0.06)
        .bounce(0.82)
        .surface_friction(0.9)
        .softness(0.25)
}

fn mode_opacity(mode: &Binding<i32>, target: i32) -> Computed<f32> {
    mode.clone()
        .map(move |current| if current == target { 1.0 } else { 0.0 })
        .computed()
}

fn background_layers(mode: &Binding<i32>) -> impl View {
    zstack((
        Color::srgb_hex("#0F172A").opacity(
            mode.clone()
                .map(|m| if matches!(m, 0 | 1) { 1.0 } else { 0.0 })
                .computed(),
        ),
        Color::srgb_hex("#1a221a").opacity(mode_opacity(mode, 2)),
        Color::from(Srgb::BLACK).opacity(
            mode.clone()
                .map(|m| if matches!(m, 3 | 4 | 6) { 1.0 } else { 0.0 })
                .computed(),
        ),
        Color::srgb_hex("#F0F4F8").opacity(mode_opacity(mode, 5)),
        Color::srgb_hex("#08111E").opacity(mode_opacity(mode, 7)),
    ))
}

fn particle_layers(mode: &Binding<i32>) -> impl View {
    zstack((
        rain().opacity(mode_opacity(mode, 0)),
        snow().opacity(mode_opacity(mode, 1)),
        fog().opacity(mode_opacity(mode, 2)),
        flame().opacity(mode_opacity(mode, 3)),
        firework().opacity(mode_opacity(mode, 4)),
        confetti_view().opacity(mode_opacity(mode, 5)),
        explosion().opacity(mode_opacity(mode, 6)),
        bounce_box().opacity(mode_opacity(mode, 7)),
    ))
}

fn particle_button(mode: &Binding<i32>, label: &'static str, target: i32, width: f32) -> impl View {
    button(text(label))
        .action(move |State(m): State<Binding<i32>>| m.set(target))
        .state(mode)
        .width(width)
}

/// Demo View
#[preview]
pub fn demo() -> impl View {
    let mode = binding(0);
    let is_confetti = mode.clone().map(|m| m == 5);

    zstack((
        background_layers(&mode),
        particle_layers(&mode),
        vstack((
            zstack((
                title_label(Color::from(Srgb::BLACK)).opacity(is_confetti.clone().select(1.0, 0.0)),
                title_label(Color::from(Srgb::WHITE)).opacity(is_confetti.select(0.0, 1.0)),
            )),
            vstack((
                hstack((
                    particle_button(&mode, "Rain", 0, 96.0),
                    particle_button(&mode, "Snow", 1, 96.0),
                    particle_button(&mode, "Fog", 2, 96.0),
                ))
                .spacing(10.0),
                hstack((
                    particle_button(&mode, "Flame", 3, 112.0),
                    particle_button(&mode, "Firework", 4, 128.0),
                ))
                .spacing(10.0),
                hstack((
                    particle_button(&mode, "Confetti", 5, 128.0),
                    particle_button(&mode, "Explosion", 6, 136.0),
                    particle_button(&mode, "Bounce Box", 7, 152.0),
                ))
                .spacing(10.0),
            ))
            .spacing(10.0),
            spacer(),
        ))
        .padding_with(EdgeInsets::all(40.0)),
    ))
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
