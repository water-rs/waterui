//! A mesh gradient animated on the GPU: a 4×4 palette warped by flowing noise.

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

use cherenkov::WorkingColor;
use nami::{Computed, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View};

use crate::color::working::from_linear_srgb;
use crate::shader_paint::ShaderPaintView;

/// Number of colours in the animated mesh palette (a 4×4 grid).
pub const ANIMATED_MESH_PALETTE_LEN: usize = 16;

/// The user uniforms the shader reads: `speed`, `warp`, two padding floats,
/// then the palette's RGB triples.
const UNIFORM_LEN: usize = 4 + ANIMATED_MESH_PALETTE_LEN * 3;

/// Configuration for an [`AnimatedMeshGradient`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimatedMeshGradientConfig {
    /// Animation speed multiplier on the engine's clock; `0.0` holds the
    /// gradient still and lets the engine idle.
    pub speed: f32,
    /// UV warp strength (controls flow intensity).
    pub warp: f32,
    /// The 4×4 mesh palette, row-major, the first row along the bottom
    /// edge. The palette is opaque: alpha is ignored.
    pub palette: [WorkingColor; ANIMATED_MESH_PALETTE_LEN],
}

nami::impl_constant!(AnimatedMeshGradientConfig);

impl AnimatedMeshGradientConfig {
    /// Sets the animation speed (must be >= 0.0).
    ///
    /// # Panics
    ///
    /// Panics when `speed` is negative or not finite.
    #[must_use]
    pub fn speed(mut self, speed: f32) -> Self {
        assert_speed(speed);
        self.speed = speed;
        self
    }

    /// Sets the warp strength (recommended 0.0..0.5).
    ///
    /// # Panics
    ///
    /// Panics when `warp` is negative or not finite.
    #[must_use]
    pub fn warp(mut self, warp: f32) -> Self {
        assert_warp(warp);
        self.warp = warp;
        self
    }

    /// Sets the 4×4 palette (row-major, first row along the bottom edge).
    #[must_use]
    pub const fn palette(mut self, palette: [WorkingColor; ANIMATED_MESH_PALETTE_LEN]) -> Self {
        self.palette = palette;
        self
    }

    /// Aqua + lavender pastel palette with soft contrast.
    #[must_use]
    pub fn aqua_bloom() -> Self {
        preset(
            0.6,
            0.24,
            [
                [0.60, 0.92, 0.98],
                [0.70, 0.90, 0.98],
                [0.78, 0.84, 0.96],
                [0.84, 0.92, 0.98],
                [0.38, 0.72, 0.92],
                [0.46, 0.64, 0.90],
                [0.82, 0.64, 0.92],
                [0.90, 0.74, 0.92],
                [0.30, 0.56, 0.86],
                [0.42, 0.52, 0.86],
                [0.78, 0.56, 0.86],
                [0.94, 0.70, 0.88],
                [0.22, 0.44, 0.78],
                [0.36, 0.46, 0.80],
                [0.62, 0.54, 0.80],
                [0.86, 0.66, 0.84],
            ],
        )
    }

    /// Soft pastel palette with gentle cyan/pink transitions.
    #[must_use]
    pub fn pastel_lagoon() -> Self {
        preset(
            0.55,
            0.16,
            [
                [0.72, 0.94, 0.98],
                [0.62, 0.90, 0.98],
                [0.72, 0.82, 0.96],
                [0.88, 0.80, 0.96],
                [0.56, 0.86, 0.97],
                [0.64, 0.80, 0.96],
                [0.80, 0.74, 0.94],
                [0.96, 0.82, 0.92],
                [0.48, 0.80, 0.95],
                [0.60, 0.72, 0.94],
                [0.82, 0.70, 0.92],
                [0.96, 0.78, 0.90],
                [0.42, 0.72, 0.92],
                [0.54, 0.66, 0.92],
                [0.76, 0.64, 0.88],
                [0.92, 0.74, 0.86],
            ],
        )
    }

    /// Bright white base with blush lavender accents.
    #[must_use]
    pub fn soft_blush() -> Self {
        preset(
            0.45,
            0.12,
            [
                [0.98, 0.98, 1.00],
                [0.96, 0.96, 0.99],
                [0.96, 0.94, 0.99],
                [0.98, 0.96, 0.98],
                [0.94, 0.92, 0.98],
                [0.92, 0.90, 0.97],
                [0.90, 0.88, 0.96],
                [0.94, 0.90, 0.96],
                [0.90, 0.86, 0.95],
                [0.88, 0.84, 0.94],
                [0.86, 0.82, 0.94],
                [0.90, 0.84, 0.94],
                [0.88, 0.80, 0.92],
                [0.86, 0.78, 0.92],
                [0.84, 0.76, 0.90],
                [0.88, 0.78, 0.90],
            ],
        )
    }

    /// Deep blue palette with soft cyan transitions.
    #[must_use]
    pub fn deep_blue() -> Self {
        preset(
            0.6,
            0.2,
            [
                [0.02, 0.06, 0.18],
                [0.04, 0.12, 0.28],
                [0.06, 0.18, 0.36],
                [0.08, 0.24, 0.42],
                [0.06, 0.20, 0.38],
                [0.10, 0.28, 0.48],
                [0.12, 0.36, 0.56],
                [0.14, 0.42, 0.62],
                [0.08, 0.26, 0.44],
                [0.12, 0.34, 0.54],
                [0.16, 0.44, 0.66],
                [0.18, 0.52, 0.74],
                [0.06, 0.22, 0.40],
                [0.10, 0.30, 0.50],
                [0.14, 0.40, 0.64],
                [0.18, 0.50, 0.78],
            ],
        )
    }

    /// The flat uniform list the shader reads.
    ///
    /// # Panics
    ///
    /// When `speed` or `warp` is negative or not finite: the public fields
    /// bypass the builders' checks, so the values are checked again here.
    fn uniforms(&self) -> Vec<f32> {
        assert_speed(self.speed);
        assert_warp(self.warp);
        let mut uniforms = Vec::with_capacity(UNIFORM_LEN);
        uniforms.extend([self.speed, self.warp, 0.0, 0.0]);
        for color in &self.palette {
            let [red, green, blue, _] = color.components;
            uniforms.extend([red, green, blue]);
        }
        uniforms
    }
}

impl Default for AnimatedMeshGradientConfig {
    fn default() -> Self {
        Self::aqua_bloom()
    }
}

/// A preset from its speed, warp and linear sRGB palette.
fn preset(
    speed: f32,
    warp: f32,
    palette: [[f32; 3]; ANIMATED_MESH_PALETTE_LEN],
) -> AnimatedMeshGradientConfig {
    AnimatedMeshGradientConfig {
        speed,
        warp,
        palette: palette.map(|rgb| from_linear_srgb(rgb, 1.0)),
    }
}

fn assert_speed(speed: f32) {
    assert!(
        speed.is_finite() && speed >= 0.0,
        "AnimatedMeshGradient speed must be finite and >= 0.0, got {speed}"
    );
}

fn assert_warp(warp: f32) {
    assert!(
        warp.is_finite() && warp >= 0.0,
        "AnimatedMeshGradient warp must be finite and >= 0.0, got {warp}"
    );
}

/// A mesh gradient animated on the GPU: a 4×4 palette warped by flowing
/// noise, advanced by the engine's frame clock.
///
/// The configuration follows a signal. A new speed, warp or palette updates
/// the shader's uniforms in place; the engine re-renders the gradient every
/// frame while `speed` is positive and stays idle while it is `0.0`.
///
/// # Layout Behavior
///
/// Stretches on both axes; constrain it with `.frame()`.
pub struct AnimatedMeshGradient {
    config: Computed<AnimatedMeshGradientConfig>,
}

impl fmt::Debug for AnimatedMeshGradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AnimatedMeshGradient")
            .finish_non_exhaustive()
    }
}

impl AnimatedMeshGradient {
    /// An animated mesh gradient whose configuration follows `config`.
    #[must_use]
    pub fn new(config: impl IntoComputed<AnimatedMeshGradientConfig>) -> Self {
        Self {
            config: config.into_computed(),
        }
    }
}

impl Default for AnimatedMeshGradient {
    fn default() -> Self {
        Self::new(AnimatedMeshGradientConfig::default())
    }
}

impl View for AnimatedMeshGradient {
    fn body(self, _env: &Environment) -> impl View {
        ShaderPaintView::new(include_str!("animated_mesh.wgsl"))
            .animated(self.config.map(|config| config.speed > 0.0))
            .uniforms(self.config.map(|config| config.uniforms()))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_uniforms_fit_the_shader_paint_budget() {
        // A shader paint takes at most 64 uniform floats.
        let uniforms = AnimatedMeshGradientConfig::aqua_bloom().uniforms();
        assert_eq!(uniforms.len(), UNIFORM_LEN);
        assert!(uniforms.len() <= 64);
    }

    #[test]
    fn the_uniforms_lead_with_speed_and_warp_then_the_palette() {
        let config = AnimatedMeshGradientConfig::deep_blue().speed(1.5).warp(0.3);
        let uniforms = config.uniforms();
        assert_eq!(&uniforms[..2], &[1.5, 0.3]);
        let [red, green, blue, _] = config.palette[15].components;
        assert_eq!(&uniforms[UNIFORM_LEN - 3..], &[red, green, blue]);
    }

    #[test]
    #[should_panic(expected = "speed must be finite and >= 0.0")]
    fn a_negative_speed_is_rejected() {
        let _ = AnimatedMeshGradientConfig::default().speed(-1.0);
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "the presets set alpha to exactly 1")]
    fn the_presets_are_opaque() {
        let config = AnimatedMeshGradientConfig::soft_blush();
        assert!(
            config
                .palette
                .iter()
                .all(|color| color.components[3] == 1.0)
        );
    }
}
