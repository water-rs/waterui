//! Gradient views: a Cherenkov [`Paint`] filling the view's bounds.
//!
//! A gradient is authored in unit space — `[0, 1]` on both axes, `(0, 0)` the
//! top-left of the view — and the backend maps it onto the bounds it lays the
//! view out at. The mesh's control points follow the same convention. A radial
//! radius is a circle, never an ellipse: on a `w` × `h` box a radius `r` is a
//! circle of `r * min(w, h)` points, so a radius of `0.5` from the centre
//! reaches the nearer edge and stops short of the farther one on a non-square
//! box.

extern crate alloc;

use alloc::vec;
use alloc::vec::Vec;
use core::f32::consts::TAU;

use crate::draw::kurbo::{Affine, Point};
use crate::draw::{
    ColorStop, LinearGradient, MeshColorInterpolation, MeshGradient, Paint, RadialGradient,
    SweepGradient, WorkingColor,
};
use waterui_core::layout::StretchAxis;
use waterui_core::{View, contract_violation};

/// The family of a gradient, for backends that map it onto a platform type.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum GradientType {
    /// Colours run along a line.
    #[default]
    Linear = 0,
    /// Colours run outward from a centre.
    Radial = 1,
    /// Colours run around a centre.
    Angular = 2,
    /// Colours are interpolated across a grid of control points.
    Mesh = 3,
}

nami::impl_constant!(GradientType);

fn stops(stops: Vec<(f32, WorkingColor)>) -> Vec<ColorStop> {
    let given = stops.len();
    if given == 0 {
        contract_violation!(
            "a gradient must contain at least one stop; a single transparent stop is used"
        );
    }
    let mut stops = stops
        .into_iter()
        .filter_map(|(offset, color)| {
            if !color
                .components
                .iter()
                .all(|component| component.is_finite())
            {
                contract_violation!(
                    "gradient stop colour must be finite, got {color:?}; the stop is dropped"
                );
                return None;
            }
            Some(ColorStop {
                offset: stop_offset(offset),
                color,
            })
        })
        .collect::<Vec<_>>();
    if stops.is_empty() {
        stops.push(ColorStop {
            offset: 0.0,
            color: WorkingColor::TRANSPARENT,
        });
    }
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

/// `offset` when it lies within `[0, 1]`; otherwise the nearest end, `0` for
/// `NaN`.
fn stop_offset(offset: f32) -> f32 {
    if (0.0..=1.0).contains(&offset) {
        return offset;
    }
    contract_violation!(
        "gradient stop position must be within [0, 1], got {offset}; the nearest end of the range is used"
    );
    if offset.is_nan() {
        0.0
    } else {
        offset.clamp(0.0, 1.0)
    }
}

fn point([x, y]: [f32; 2], name: &str) -> Point {
    let finite = |value: f32, axis: &str| {
        if value.is_finite() {
            f64::from(value)
        } else {
            contract_violation!("{name}.{axis} must be finite; 0 is used");
            0.0
        }
    };
    Point::new(finite(x, "x"), finite(y, "y"))
}

/// The mesh a gradient with an invalid grid draws: one transparent patch.
fn empty_mesh() -> MeshGradient {
    MeshGradient::new(
        1,
        1,
        vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
            Point::new(1.0, 1.0),
        ],
        vec![WorkingColor::TRANSPARENT; 4],
    )
    .expect("a one-patch grid with four points and four colours is a valid mesh")
}

/// The mesh paint over a `columns` × `rows` grid of unit-space control points
/// and their colours, row-major.
///
/// A grid with fewer than two vertices on a side, or a list that does not
/// hold exactly `columns * rows` entries, is a contract violation: the mesh
/// is one transparent patch.
pub(super) fn mesh_paint(
    columns: u32,
    rows: u32,
    points: Vec<[f32; 2]>,
    colors: Vec<WorkingColor>,
    smooths_colors: bool,
) -> MeshGradient {
    if columns < 2 || rows < 2 {
        contract_violation!(
            "mesh gradients need at least a 2x2 grid of vertices, got {columns}x{rows}; the mesh is one transparent patch"
        );
        return empty_mesh();
    }
    let vertices = columns as usize * rows as usize;
    if points.len() != vertices {
        contract_violation!(
            "mesh gradients require exactly columns*rows control points, got {} for {vertices}; the mesh is one transparent patch",
            points.len()
        );
        return empty_mesh();
    }
    if colors.len() != vertices {
        contract_violation!(
            "mesh gradients require exactly columns*rows colours, got {} for {vertices}; the mesh is one transparent patch",
            colors.len()
        );
        return empty_mesh();
    }
    let points = points
        .into_iter()
        .map(|position| point(position, "mesh gradient vertex"))
        .collect();
    MeshGradient::new(columns - 1, rows - 1, points, colors)
        .expect("the checks above guarantee a non-empty grid with one entry per vertex")
        .interpolation(if smooths_colors {
            MeshColorInterpolation::Smoothstep
        } else {
            MeshColorInterpolation::Linear
        })
}

/// A gradient view: a [`Paint`] in unit space that fills the view's bounds.
///
/// # Layout Behavior
///
/// A gradient is a greedy view: it expands to fill all available space on
/// both axes. Constrain it with `.frame()` or use it as a background.
#[derive(Debug, Clone)]
pub struct Gradient {
    gradient_type: GradientType,
    paint: Paint,
}

impl Gradient {
    /// A gradient whose colours run along the line from `start` to `end`.
    #[must_use]
    pub fn linear(colors: Vec<(f32, WorkingColor)>, start: [f32; 2], end: [f32; 2]) -> Self {
        let mut gradient = LinearGradient::new(
            point(start, "linear gradient start"),
            point(end, "linear gradient end"),
        );
        gradient.stops = stops(colors);
        Self {
            gradient_type: GradientType::Linear,
            paint: Paint::Linear(gradient),
        }
    }

    /// A gradient whose colours run outward from `center`, between two radii.
    ///
    /// `center` is in unit space. Both radii are fractions of the shorter side
    /// of the box the view is laid out at: on a `w` × `h` box each radius `r`
    /// is a circle of `r * min(w, h)` points around `center` mapped onto the
    /// box. A non-square box does not stretch the circle into an ellipse.
    ///
    /// A start radius that is negative or not finite is a contract violation
    /// and is `0`; an end radius that is not finite or not above `0` is one
    /// too, and is `0.5`, the circle that reaches the nearer edge from the
    /// centre.
    #[must_use]
    pub fn radial(
        colors: Vec<(f32, WorkingColor)>,
        center: [f32; 2],
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        let start_radius = if start_radius.is_finite() && start_radius >= 0.0 {
            start_radius
        } else {
            contract_violation!(
                "radial gradient start radius must be finite and >= 0, got {start_radius}; 0 is used"
            );
            0.0
        };
        let end_radius = if end_radius.is_finite() && end_radius > 0.0 {
            end_radius
        } else {
            contract_violation!(
                "radial gradient end radius must be finite and > 0, got {end_radius}; 0.5 is used"
            );
            0.5
        };
        let center = point(center, "radial gradient center");
        let mut gradient = RadialGradient::two_point(
            center,
            f64::from(start_radius),
            center,
            f64::from(end_radius),
        );
        gradient.stops = stops(colors);
        Self {
            gradient_type: GradientType::Radial,
            paint: Paint::Radial(gradient),
        }
    }

    /// A gradient whose colours run around `center`, from one angle to the
    /// other, in radians.
    ///
    /// An angle that is not finite, or a sweep that is not positive or is
    /// more than a full turn, is a contract violation: the gradient runs a
    /// full turn from the start angle (`0` when it is not finite).
    #[must_use]
    pub fn angular(
        colors: Vec<(f32, WorkingColor)>,
        center: [f32; 2],
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        let (start_angle, end_angle) = if !(start_angle.is_finite() && end_angle.is_finite()) {
            contract_violation!(
                "angular gradient angles must be finite, got {start_angle} and {end_angle}; a full turn from 0 is used"
            );
            (0.0, TAU)
        } else if end_angle - start_angle <= 0.0 {
            contract_violation!(
                "angular gradient sweep must be positive, got {}; a full turn is used",
                end_angle - start_angle
            );
            (start_angle, start_angle + TAU)
        } else if end_angle - start_angle > TAU {
            contract_violation!(
                "angular gradient sweep must be <= TAU, got {}; a full turn is used",
                end_angle - start_angle
            );
            (start_angle, start_angle + TAU)
        } else {
            (start_angle, end_angle)
        };
        let mut gradient = SweepGradient::new(
            point(center, "angular gradient center"),
            f64::from(start_angle),
            f64::from(end_angle),
        );
        gradient.stops = stops(colors);
        Self {
            gradient_type: GradientType::Angular,
            paint: Paint::Sweep(gradient),
        }
    }

    /// A gradient interpolated across a `columns` × `rows` grid of control
    /// vertices in unit space, row-major.
    ///
    /// `smooths_colors` eases the colour weights across each patch
    /// (`t * t * (3 - 2t)` on both patch coordinates) instead of blending them
    /// bilinearly, which hides the grid's seams.
    ///
    /// A grid with fewer than two vertices on a side, or a `vertices` that
    /// does not hold exactly `columns * rows` entries, is a contract
    /// violation: the mesh is one transparent patch.
    #[must_use]
    pub fn mesh(
        columns: u32,
        rows: u32,
        vertices: Vec<([f32; 2], WorkingColor)>,
        smooths_colors: bool,
    ) -> Self {
        let (points, colors): (Vec<[f32; 2]>, Vec<WorkingColor>) = vertices.into_iter().unzip();
        Self {
            gradient_type: GradientType::Mesh,
            paint: Paint::Mesh(mesh_paint(columns, rows, points, colors, smooths_colors)),
        }
    }

    /// The family of this gradient.
    #[must_use]
    pub const fn gradient_type(&self) -> GradientType {
        self.gradient_type
    }

    /// The paint in unit space.
    #[must_use]
    pub const fn paint(&self) -> &Paint {
        &self.paint
    }

    /// The transform that maps unit space onto a `width` × `height` box.
    #[must_use]
    pub fn transform_to(width: f32, height: f32) -> Affine {
        Affine::scale_non_uniform(f64::from(width), f64::from(height))
    }
}

impl waterui_core::NativeView for Gradient {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

impl View for Gradient {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        if matches!(self.paint(), Paint::Mesh(_)) {
            waterui_core::AnyView::new(super::mesh::static_mesh_view(self.paint))
        } else {
            waterui_core::AnyView::new(waterui_core::Native::new(self))
        }
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "sorting preserves these exact input offsets"
    )]
    fn stops_are_sorted_by_offset() {
        let gradient = Gradient::linear(
            vec![(1.0, WorkingColor::WHITE), (0.0, WorkingColor::BLACK)],
            [0.0, 0.0],
            [1.0, 0.0],
        );
        let Paint::Linear(linear) = gradient.paint() else {
            panic!("a linear gradient is a linear paint");
        };
        assert_eq!(linear.stops[0].offset, 0.0);
        assert_eq!(linear.stops[1].offset, 1.0);
    }

    fn linear_stops(gradient: &Gradient) -> &[ColorStop] {
        let Paint::Linear(linear) = gradient.paint() else {
            panic!("a linear gradient is a linear paint");
        };
        &linear.stops
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "within [0, 1]"))]
    fn an_offset_past_one_is_clamped() {
        let gradient = Gradient::linear(
            vec![(1.5, WorkingColor::WHITE), (f32::NAN, WorkingColor::BLACK)],
            [0.0, 0.0],
            [1.0, 0.0],
        );
        let offsets: Vec<f32> = linear_stops(&gradient)
            .iter()
            .map(|stop| stop.offset)
            .collect();
        assert_eq!(offsets, [0.0, 1.0]);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "at least one stop"))]
    fn a_gradient_without_stops_is_one_transparent_stop() {
        let gradient = Gradient::linear(Vec::new(), [0.0, 0.0], [1.0, 0.0]);
        assert_eq!(
            linear_stops(&gradient),
            [ColorStop {
                offset: 0.0,
                color: WorkingColor::TRANSPARENT
            }]
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "colour must be finite"))]
    fn a_stop_with_a_non_finite_colour_is_dropped() {
        let broken = WorkingColor::new([f32::NAN, 0.0, 0.0, 1.0]);
        let gradient = Gradient::linear(
            vec![(0.0, broken), (1.0, WorkingColor::WHITE)],
            [0.0, 0.0],
            [1.0, 0.0],
        );
        assert_eq!(
            linear_stops(&gradient),
            [ColorStop {
                offset: 1.0,
                color: WorkingColor::WHITE
            }]
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "must be finite"))]
    fn a_non_finite_point_is_the_origin() {
        let gradient = Gradient::linear(
            vec![(0.0, WorkingColor::BLACK)],
            [f32::INFINITY, 0.5],
            [1.0, 0.0],
        );
        let Paint::Linear(linear) = gradient.paint() else {
            panic!("a linear gradient is a linear paint");
        };
        assert_eq!(linear.start, Point::new(0.0, 0.5));
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "sweep must be positive"))]
    fn a_backwards_angular_sweep_is_a_full_turn() {
        let gradient = Gradient::angular(vec![(0.0, WorkingColor::BLACK)], [0.5, 0.5], 1.0, 0.5);
        let Paint::Sweep(sweep) = gradient.paint() else {
            panic!("an angular gradient is a sweep paint");
        };
        assert!((sweep.end_angle - sweep.start_angle - f64::from(TAU)).abs() < 1e-6);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "end radius"))]
    fn an_empty_radial_end_radius_is_half_the_shorter_side() {
        let gradient = Gradient::radial(vec![(0.0, WorkingColor::BLACK)], [0.5, 0.5], 0.0, 0.0);
        let Paint::Radial(radial) = gradient.paint() else {
            panic!("a radial gradient is a radial paint");
        };
        assert!((radial.end_radius - 0.5).abs() < 1e-9);
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "2x2 grid"))]
    fn a_degenerate_mesh_is_one_transparent_patch() {
        let gradient = Gradient::mesh(1, 2, Vec::new(), true);
        let Paint::Mesh(mesh) = gradient.paint() else {
            panic!("a mesh gradient is a mesh paint");
        };
        assert_eq!((mesh.columns(), mesh.rows()), (1, 1));
        assert!(
            mesh.colors()
                .iter()
                .all(|color| *color == WorkingColor::TRANSPARENT)
        );
    }

    #[test]
    fn a_mesh_carries_its_grid_and_interpolation() {
        let vertices = vec![
            ([0.0, 0.0], WorkingColor::BLACK),
            ([1.0, 0.0], WorkingColor::WHITE),
            ([0.0, 1.0], WorkingColor::WHITE),
            ([1.0, 1.0], WorkingColor::BLACK),
        ];
        for (smooths, mode) in [
            (true, MeshColorInterpolation::Smoothstep),
            (false, MeshColorInterpolation::Linear),
        ] {
            let gradient = Gradient::mesh(2, 2, vertices.clone(), smooths);
            assert_eq!(gradient.gradient_type(), GradientType::Mesh);
            let Paint::Mesh(mesh) = gradient.paint() else {
                panic!("a mesh gradient is a mesh paint");
            };
            assert_eq!((mesh.columns(), mesh.rows()), (1, 1));
            assert_eq!(mesh.interpolation_mode(), mode);
        }
    }
}
