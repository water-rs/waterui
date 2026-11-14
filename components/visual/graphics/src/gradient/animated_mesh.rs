//! A 4×4 mesh gradient whose points and colours move with the canvas clock.

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

use nami::{Computed, SignalExt};
use waterui_core::layout::{Point, Rect, Size, StretchAxis};
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View, contract_violation};

use crate::canvas::{Canvas, MeshGradient};
use crate::color::WorkingColor;
use crate::color::working::from_linear_srgb;

/// Number of colours in the animated mesh palette (a 4×4 grid).
pub const ANIMATED_MESH_PALETTE_LEN: usize = 16;

/// Vertices along one side of the mesh.
const SIDE: usize = 4;

/// Where the mesh's vertices rest along each axis, in unit space.
const REST: [f32; SIDE] = [0.0, 1.0 / 3.0, 2.0 / 3.0, 1.0];

/// Configuration for an [`AnimatedMeshGradient`].
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub struct AnimatedMeshGradientConfig {
    /// Animation speed multiplier on the engine's clock; `0.0` holds the
    /// gradient still and lets the engine idle.
    ///
    /// The flow's phase is the engine clock times `speed`, so a new speed
    /// takes effect from the current clock rather than continuing the old
    /// phase.
    speed: f32,
    /// UV warp strength (controls flow intensity).
    warp: f32,
    /// The 4×4 mesh palette, row-major, the first row along the bottom
    /// edge. The palette is opaque: alpha is ignored.
    palette: [WorkingColor; ANIMATED_MESH_PALETTE_LEN],
}

impl AnimatedMeshGradientConfig {
    /// Animation speed multiplier on the engine's clock; `0.0` holds the
    /// gradient still and lets the engine idle.
    ///
    /// The flow's phase is the engine clock times `speed`, so a new speed
    /// takes effect from the current clock rather than continuing the old
    /// phase.
    #[must_use]
    pub const fn get_speed(&self) -> f32 {
        self.speed
    }

    /// UV warp strength (controls flow intensity).
    #[must_use]
    pub const fn get_warp(&self) -> f32 {
        self.warp
    }

    /// The 4×4 mesh palette, row-major, the first row along the bottom
    /// edge. The palette is opaque: alpha is ignored.
    #[must_use]
    pub const fn get_palette(&self) -> [WorkingColor; ANIMATED_MESH_PALETTE_LEN] {
        self.palette
    }
}

nami::impl_constant!(AnimatedMeshGradientConfig);

impl AnimatedMeshGradientConfig {
    /// Sets the animation speed (must be >= 0.0).
    ///
    /// A negative or non-finite `speed` is a contract violation: the speed is
    /// `0.0`.
    #[must_use]
    pub fn speed(mut self, speed: f32) -> Self {
        self.speed = checked_speed(speed);
        self
    }

    /// Sets the warp strength (recommended 0.0..0.5).
    ///
    /// A negative or non-finite `warp` is a contract violation: the warp is
    /// `0.0`.
    #[must_use]
    pub fn warp(mut self, warp: f32) -> Self {
        self.warp = checked_warp(warp);
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

    /// The mesh at `elapsed` seconds on the canvas clock, laid over `size`.
    ///
    /// The vertices rest on an even 4×4 grid and are displaced by a smooth
    /// two-stage flow field, drifting at the rates the shader's noise did and
    /// scaled by `warp`; the border stays pinned. Each vertex takes the
    /// palette's colour at its displaced position, dimmed toward the corners
    /// by the shader's vignette.
    fn mesh(&self, size: Size, elapsed: f64) -> MeshGradient {
        let phase = flow_phase(elapsed, self.speed);
        let aspect = if size.height > 0.0 {
            size.width / size.height
        } else {
            1.0
        };
        let mut points = Vec::with_capacity(ANIMATED_MESH_PALETTE_LEN);
        let mut colors = Vec::with_capacity(ANIMATED_MESH_PALETTE_LEN);
        // Mesh rows run top to bottom; the palette's first row is the bottom.
        for &v in REST.iter().rev() {
            for &u in &REST {
                let [dx, dy] = displacement([u, v], aspect, phase, self.warp);
                let (x, y) = (u + dx, v + dy);
                points.push(Point::new(x * size.width, (1.0 - y) * size.height));
                let [red, green, blue, _] = self.palette_at(x, y);
                let shade = 0.04f32.mul_add(vignette(u, v, aspect), 0.96);
                colors.push(WorkingColor::new([
                    red * shade,
                    green * shade,
                    blue * shade,
                    1.0,
                ]));
            }
        }
        MeshGradient::new(3, 3, points, colors)
    }

    /// The palette eased across its cells at unit-space `(u, v)`, `v` up.
    fn palette_at(&self, u: f32, v: f32) -> [f32; 4] {
        let (column, tx) = cell(u.clamp(0.0, 1.0) * 3.0);
        let (row, ty) = cell(v.clamp(0.0, 1.0) * 3.0);
        let (tx, ty) = (ease(tx), ease(ty));
        let at = |row: usize, column: usize| self.palette[row * SIDE + column].components;
        let bottom = mix(at(row, column), at(row, column + 1), tx);
        let top = mix(at(row + 1, column), at(row + 1, column + 1), tx);
        mix(bottom, top, ty)
    }
}

/// The flow's phase: the clock times `speed`, as the shader's `t`.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the product is formed in f64 and narrowed once; f32 resolves it to milliseconds for hours"
)]
fn flow_phase(elapsed: f64, speed: f32) -> f32 {
    (elapsed * f64::from(speed)) as f32
}

/// A smooth stand-in for the shader's noise: three crossed sines in
/// `[-1, 1]`.
fn wave(x: f32, y: f32) -> f32 {
    (4.4f32.mul_add(x, 0.3).sin() + 3.7f32.mul_add(y, 1.3).sin() + 2.9f32.mul_add(x + y, 0.7).sin())
        / 3.0
}

/// `p` moved by `by` scaled with `k`.
const fn advect(p: [f32; 2], by: [f32; 2], k: f32) -> [f32; 2] {
    [by[0].mul_add(k, p[0]), by[1].mul_add(k, p[1])]
}

/// One drifting wave: `scale` applied to `p`, then carried by `drift` per
/// unit of phase `t`, from `offset`.
fn drifting(p: [f32; 2], scale: f32, drift: [f32; 2], t: f32, offset: [f32; 2]) -> f32 {
    wave(
        scale.mul_add(p[0], drift[0].mul_add(t, offset[0])),
        scale.mul_add(p[1], drift[1].mul_add(t, offset[1])),
    ) * 0.3
}

/// The shader's two-stage flow at centred, aspect-scaled `p`: a first flow
/// from two drifting waves, then a finer one sampled through the first.
fn flow(p: [f32; 2], t: f32) -> [f32; 2] {
    let first = [
        drifting(p, 1.8, [0.12, -0.10], t, [0.0, 0.0]),
        drifting(p, 2.1, [-0.09, 0.11], t, [2.0, -1.6]),
    ];
    let second = [
        drifting(advect(p, first, 0.6), 3.0, [0.18, 0.14], t, [0.0, 0.0]),
        drifting(advect(p, first, -0.5), 3.6, [-0.15, 0.12], t, [0.0, 0.0]),
    ];
    [
        second[0].mul_add(0.45, first[0]),
        second[1].mul_add(0.45, first[1]),
    ]
}

/// How far the vertex resting at unit-space `rest` moves, in unit space.
/// Zero on the border, where the mesh stays pinned to the box.
fn displacement(rest: [f32; 2], aspect: f32, phase: f32, warp: f32) -> [f32; 2] {
    let [u, v] = rest;
    let edge = smoothstep(0.0, 0.12, u.min(1.0 - u).min(v.min(1.0 - v)));
    let [fx, fy] = flow([(u - 0.5) * aspect, v - 0.5], phase);
    [fx * warp * edge, fy * warp * edge]
}

/// The shader's vignette weight at unit-space `(u, v)`.
fn vignette(u: f32, v: f32, aspect: f32) -> f32 {
    smoothstep(1.1, 0.18, ((u - 0.5) * aspect).hypot(v - 0.5))
}

/// GLSL's `smoothstep`, edges in either order.
fn smoothstep(from: f32, to: f32, x: f32) -> f32 {
    ease(((x - from) / (to - from)).clamp(0.0, 1.0))
}

/// The cubic ease the shader applied across each cell.
fn ease(t: f32) -> f32 {
    t * t * 2.0f32.mul_add(-t, 3.0)
}

/// The palette cell `p` (in `0.0..=3.0`) lies in and the fraction across it.
fn cell(p: f32) -> (usize, f32) {
    let (index, start) = if p >= 2.0 {
        (2, 2.0)
    } else if p >= 1.0 {
        (1, 1.0)
    } else {
        (0, 0.0)
    };
    (index, p - start)
}

/// Linear interpolation of two colours' components.
fn mix(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    core::array::from_fn(|i| (b[i] - a[i]).mul_add(t, a[i]))
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

/// `speed` when valid; otherwise `0.0`, which holds the gradient still.
fn checked_speed(speed: f32) -> f32 {
    if speed.is_finite() && speed >= 0.0 {
        return speed;
    }
    contract_violation!(
        "AnimatedMeshGradient speed must be finite and >= 0.0, got {speed}; 0.0 is used"
    );
    0.0
}

/// `warp` when valid; otherwise `0.0`, which leaves the vertices on the grid.
fn checked_warp(warp: f32) -> f32 {
    if warp.is_finite() && warp >= 0.0 {
        return warp;
    }
    contract_violation!(
        "AnimatedMeshGradient warp must be finite and >= 0.0, got {warp}; 0.0 is used"
    );
    0.0
}

/// A 4×4 mesh gradient whose vertices and colours flow with the canvas
/// clock, drawn through [`Canvas`] on every backend.
///
/// The configuration follows a signal. A new speed, warp or palette updates
/// the mesh in place; the gradient presents a frame every refresh while
/// `speed` is positive and presents none while it is `0.0`.
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
        let config = self.config;
        Canvas::new(move |cx| {
            let size = cx.size();
            let clock = cx.time();
            // Only an animating gradient observes the clock, so a gradient
            // held at speed 0 presents no frames; the region re-records when
            // the speed crosses zero.
            let animating = config.map(|config| config.speed > 0.0);
            cx.scope(animating, move |cx, animating: &bool| {
                let elapsed: Computed<f64> = if *animating {
                    clock.map(|time| time.as_secs_f64()).computed()
                } else {
                    Computed::constant(0.0)
                };
                let mesh = size
                    .zip(&config)
                    .zip(&elapsed)
                    .map(|((size, config), elapsed)| config.mesh(size, elapsed))
                    .computed();
                cx.fill(size.map(Rect::from_size), mesh);
            });
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOX: Size = Size::new(300.0, 200.0);

    #[test]
    fn the_mesh_is_four_by_four_vertices() {
        let mesh = AnimatedMeshGradientConfig::aqua_bloom().mesh(BOX, 1.0);
        assert_eq!((mesh.columns(), mesh.rows()), (3, 3));
        assert_eq!(mesh.points().len(), ANIMATED_MESH_PALETTE_LEN);
        assert_eq!(mesh.colors().len(), ANIMATED_MESH_PALETTE_LEN);
    }

    #[test]
    fn the_border_stays_pinned_while_the_interior_moves() {
        let config = AnimatedMeshGradientConfig::deep_blue();
        let (early, late) = (config.mesh(BOX, 0.0), config.mesh(BOX, 7.0));
        let interior = [5, 6, 9, 10];
        for (index, (a, b)) in early.points().iter().zip(late.points()).enumerate() {
            if interior.contains(&index) {
                assert!(
                    (a.x - b.x).abs() + (a.y - b.y).abs() > 1.0,
                    "interior vertex {index} moved from {a:?} to {b:?}"
                );
            } else {
                assert!(
                    (a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3,
                    "border vertex {index} moved from {a:?} to {b:?}"
                );
            }
        }
        assert_ne!(early.colors(), late.colors(), "the colours move with time");
    }

    #[test]
    fn a_held_gradient_is_the_mesh_at_phase_zero() {
        let held = AnimatedMeshGradientConfig::aqua_bloom().speed(0.0);
        assert_eq!(held.mesh(BOX, 0.0), held.mesh(BOX, 9.0));
    }

    #[test]
    fn a_zero_warp_leaves_the_vertices_on_the_grid() {
        let mesh = AnimatedMeshGradientConfig::aqua_bloom()
            .warp(0.0)
            .mesh(BOX, 3.0);
        let corner = mesh.points()[SIDE * SIDE - 1];
        assert!((corner.x - BOX.width).abs() < 1e-3 && (corner.y - BOX.height).abs() < 1e-3);
        let inner = mesh.points()[5];
        assert!((inner.x - BOX.width / 3.0).abs() < 1e-3);
        assert!((inner.y - BOX.height / 3.0).abs() < 1e-3);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "speed must be finite and >= 0.0")
    )]
    fn a_negative_speed_holds_the_gradient_still() {
        let config = AnimatedMeshGradientConfig::default().speed(-1.0);
        assert_eq!(config.speed.to_bits(), 0.0_f32.to_bits());
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "warp must be finite and >= 0.0")
    )]
    fn a_non_finite_warp_is_no_warp() {
        let config = AnimatedMeshGradientConfig::default().warp(f32::NAN);
        assert_eq!(config.warp.to_bits(), 0.0_f32.to_bits());
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
