//! Gradient views: a Cherenkov [`Paint`] filling the view's bounds.
//!
//! A gradient is authored in unit space — `[0, 1]` on both axes, `(0, 0)` the
//! top-left of the view — and the backend maps it onto the bounds it lays the
//! view out at. Radii and the mesh's control points follow the same
//! convention: a radius of `0.5` reaches the nearer edge from the centre.

extern crate alloc;

use alloc::vec::Vec;

use cherenkov::kurbo::{Affine, Point};
use cherenkov::{
    ColorStop, LinearGradient, MeshGradient, Paint, RadialGradient, SweepGradient, WorkingColor,
};
use waterui_core::View;
use waterui_core::layout::StretchAxis;

/// The family of a gradient, for backends that map it onto a platform type.
#[repr(u32)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
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
    assert!(
        !stops.is_empty(),
        "a gradient must contain at least one stop"
    );
    let mut stops = stops
        .into_iter()
        .map(|(offset, color)| {
            assert!(
                offset.is_finite() && (0.0..=1.0).contains(&offset),
                "gradient stop position must be within [0, 1], got {offset}"
            );
            assert!(
                color
                    .components
                    .iter()
                    .all(|component| component.is_finite()),
                "gradient stop colour must be finite, got {color:?}"
            );
            ColorStop { offset, color }
        })
        .collect::<Vec<_>>();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

fn point([x, y]: [f32; 2], name: &str) -> Point {
    assert!(x.is_finite(), "{name}.x must be finite");
    assert!(y.is_finite(), "{name}.y must be finite");
    Point::new(f64::from(x), f64::from(y))
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
    /// # Panics
    /// When a radius is negative or not finite.
    #[must_use]
    pub fn radial(
        colors: Vec<(f32, WorkingColor)>,
        center: [f32; 2],
        start_radius: f32,
        end_radius: f32,
    ) -> Self {
        assert!(
            start_radius.is_finite() && start_radius >= 0.0,
            "radial gradient start radius must be finite and >= 0"
        );
        assert!(
            end_radius.is_finite() && end_radius > 0.0,
            "radial gradient end radius must be finite and > 0"
        );
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
    /// # Panics
    /// When an angle is not finite.
    #[must_use]
    pub fn angular(
        colors: Vec<(f32, WorkingColor)>,
        center: [f32; 2],
        start_angle: f32,
        end_angle: f32,
    ) -> Self {
        assert!(
            start_angle.is_finite() && end_angle.is_finite(),
            "angular gradient angles must be finite"
        );
        let sweep = end_angle - start_angle;
        assert!(sweep > 0.0, "angular gradient sweep must be positive");
        assert!(
            sweep <= core::f32::consts::TAU,
            "angular gradient sweep must be <= TAU"
        );
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
    /// # Panics
    /// When the grid has fewer than two vertices on a side or `vertices`
    /// does not hold exactly `columns * rows` entries.
    #[must_use]
    pub fn mesh(columns: u32, rows: u32, vertices: Vec<([f32; 2], WorkingColor)>) -> Self {
        assert!(
            columns >= 2 && rows >= 2,
            "mesh gradients need at least a 2x2 grid of vertices"
        );
        assert_eq!(
            vertices.len(),
            (columns * rows) as usize,
            "mesh gradients require exactly columns*rows vertices"
        );
        let (points, colors): (Vec<Point>, Vec<WorkingColor>) = vertices
            .into_iter()
            .map(|(position, color)| (point(position, "mesh gradient vertex"), color))
            .unzip();
        Self {
            gradient_type: GradientType::Mesh,
            paint: Paint::Mesh(MeshGradient::new(columns - 1, rows - 1, points, colors)),
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
        waterui_core::Native::new(self)
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
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

    #[test]
    #[should_panic(expected = "within [0, 1]")]
    fn an_offset_past_one_is_rejected() {
        let _ = Gradient::linear(vec![(1.5, WorkingColor::WHITE)], [0.0, 0.0], [1.0, 0.0]);
    }

    #[test]
    fn a_mesh_carries_its_grid() {
        let gradient = Gradient::mesh(
            2,
            2,
            vec![
                ([0.0, 0.0], WorkingColor::BLACK),
                ([1.0, 0.0], WorkingColor::WHITE),
                ([0.0, 1.0], WorkingColor::WHITE),
                ([1.0, 1.0], WorkingColor::BLACK),
            ],
        );
        assert_eq!(gradient.gradient_type(), GradientType::Mesh);
        assert!(matches!(gradient.paint(), Paint::Mesh(_)));
    }
}
