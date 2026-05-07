//! High-performance GPU particle system for `WaterUI`.
//!
//! This crate provides a GPU-accelerated particle system using `wgpu` Compute Shaders
//! for simulation and instanced rendering for visualization.
//!
//! # Example: Rain Effect
//!
//! ```ignore
//! use waterui_particle::ParticleSystem;
//! use waterui::graphics::Color;
//! use std::f32::consts::PI;
//!
//! let rain = ParticleSystem::new(5000)
//!     .emit_from_rect(1.5, 0.0)
//!     .at(0.5, -0.05)
//!     .rate(800.0)
//!     .life(0.8..1.3)
//!     .speed(1.8..2.2)
//!     .angle(PI * 0.49..PI * 0.51)
//!     .size(0.002..0.004)
//!     .color(Color::srgb(255, 255, 255).with_opacity(0.5), Color::transparent())
//!     .stretch_with_velocity()
//!     .gravity(0.0, 2.5);
//! ```

mod config;
mod gpu;
mod renderer;
mod shaders;
mod system;

pub use config::{BlendMode, EmitterShape, ParticleShape};
pub use system::{ParticleSystem, particles};
