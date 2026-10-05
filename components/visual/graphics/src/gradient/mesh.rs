//! A mesh gradient whose colours and control points follow signals.

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

use crate::draw::kurbo::Rect;
use crate::draw::{Draw as _, Paint, Recorder, WorkingColor};
use nami::map::map;
use nami::{Computed, Signal, SignalExt};
use waterui_core::layout::StretchAxis;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View};

use super::paint::{Gradient, mesh_paint};
use crate::scene::resources::RecordingResources;
use crate::scene_view::{SceneContent, SceneView};

/// A mesh gradient over a `columns` × `rows` grid of vertices whose colours,
/// control points and interpolation follow signals.
///
/// The vertices are row-major in unit space, `(0, 0)` at the top-left. Until
/// [`points`](Self::points) says otherwise they sit on an even grid, the
/// first at the top-left corner and the last at the bottom-right.
///
/// A change to any input replaces the gradient's paint in the recorded scene;
/// the view is not rebuilt, so the gradient can animate at frame rate from a
/// signal.
///
/// # Layout Behavior
///
/// A mesh gradient is a greedy view: it expands to fill all available space
/// on both axes. Constrain it with `.frame()` or use it as a background.
///
/// # Panics
///
/// When the signals deliver a number of colours or control points other than
/// `columns * rows`, or a control point that is not finite.
pub struct MeshGradient {
    columns: u32,
    rows: u32,
    colors: Computed<Vec<WorkingColor>>,
    points: Computed<Vec<[f32; 2]>>,
    smooths_colors: Computed<bool>,
}

impl fmt::Debug for MeshGradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MeshGradient")
            .field("columns", &self.columns)
            .field("rows", &self.rows)
            .finish_non_exhaustive()
    }
}

impl MeshGradient {
    /// A mesh gradient over an even `columns` × `rows` grid whose vertex
    /// colours, row-major, follow `colors`.
    ///
    /// Colours are eased across each patch; see
    /// [`smooths_colors`](Self::smooths_colors).
    ///
    /// # Panics
    ///
    /// When the grid has fewer than two vertices on a side.
    #[must_use]
    pub fn new<C>(columns: u32, rows: u32, colors: C) -> Self
    where
        C: Signal + 'static,
        C::Output: IntoIterator<Item = WorkingColor>,
    {
        assert!(
            columns >= 2 && rows >= 2,
            "mesh gradients need at least a 2x2 grid of vertices"
        );
        Self {
            columns,
            rows,
            colors: Computed::new(map(colors, |colors: C::Output| {
                colors.into_iter().collect::<Vec<_>>()
            })),
            points: nami::constant(even_grid(columns, rows)).computed(),
            smooths_colors: nami::constant(true).computed(),
        }
    }

    /// Places the vertices at the unit-space control points `points`
    /// delivers, row-major.
    #[must_use]
    pub fn points<P>(mut self, points: P) -> Self
    where
        P: Signal + 'static,
        P::Output: IntoIterator<Item = [f32; 2]>,
    {
        self.points = Computed::new(map(points, |points: P::Output| {
            points.into_iter().collect::<Vec<_>>()
        }));
        self
    }

    /// Whether colours are eased across each patch (`t * t * (3 - 2t)` on
    /// both patch coordinates) rather than blended bilinearly. On by default:
    /// easing hides the grid's seams.
    #[must_use]
    pub fn smooths_colors(mut self, smooths: impl IntoComputed<bool>) -> Self {
        self.smooths_colors = smooths.into_computed();
        self
    }

    /// The mesh paint, in unit space, following every input.
    fn into_paint(self) -> Computed<Paint> {
        let Self {
            columns,
            rows,
            colors,
            points,
            smooths_colors,
        } = self;
        colors
            .zip(&points)
            .zip(&smooths_colors)
            .map(move |((colors, points), smooths_colors)| {
                Paint::Mesh(mesh_paint(columns, rows, points, colors, smooths_colors))
            })
            .computed()
    }
}

/// Where vertex `index` of `count` sits along an even grid axis, in `[0, 1]`.
#[expect(
    clippy::cast_precision_loss,
    reason = "grid indices are far below 2^24, where every u32 is an exact f32"
)]
fn grid_fraction(index: u32, count: u32) -> f32 {
    index as f32 / (count - 1) as f32
}

/// Row-major unit-space points of an even `columns` × `rows` grid.
fn even_grid(columns: u32, rows: u32) -> Vec<[f32; 2]> {
    (0..rows)
        .flat_map(|row| {
            (0..columns)
                .map(move |column| [grid_fraction(column, columns), grid_fraction(row, rows)])
        })
        .collect()
}

impl View for MeshGradient {
    fn body(self, _env: &Environment) -> impl View {
        SceneView::new(MeshContent {
            paint: self.into_paint(),
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Fills the view with the live mesh paint, authored in unit space.
struct MeshContent {
    paint: Computed<Paint>,
}

/// The self-drawn realization of a mesh gradient whose paint never changes.
///
/// There is no native mesh primitive to bridge on every platform — Android
/// has none — so a [`Gradient`] carrying a mesh resolves to engine content
/// here rather than to a `Native<Gradient>` payload a backend would draw.
pub fn static_mesh_view(paint: Paint) -> SceneView {
    SceneView::new(MeshContent {
        paint: nami::constant(paint).computed(),
    })
}

impl SceneContent for MeshContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> bool {
        let paint = self.paint.clone();
        recorder.transform(Gradient::transform_to(width, height), |recorder| {
            recorder.fill(Rect::new(0.0, 0.0, 1.0, 1.0), paint);
        });
        false
    }

    fn rebuild_for_engine(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::MeshColorInterpolation;
    use nami::Binding;

    fn mesh(paint: &Computed<Paint>) -> crate::draw::MeshGradient {
        let Paint::Mesh(mesh) = paint.snapshot() else {
            panic!("a mesh gradient records a mesh paint");
        };
        mesh
    }

    #[test]
    fn the_default_grid_is_even_and_spans_the_unit_square() {
        assert_eq!(
            even_grid(3, 2),
            vec![
                [0.0, 0.0],
                [0.5, 0.0],
                [1.0, 0.0],
                [0.0, 1.0],
                [0.5, 1.0],
                [1.0, 1.0]
            ]
        );
    }

    #[test]
    fn the_paint_follows_its_signals() {
        let colors = Binding::container(vec![WorkingColor::BLACK; 4]);
        let points = Binding::container(even_grid(2, 2));
        let smooths = Binding::container(true);
        let paint = MeshGradient::new(2, 2, colors.clone())
            .points(points.clone())
            .smooths_colors(smooths.clone())
            .into_paint();
        assert_eq!(
            mesh(&paint).interpolation_mode(),
            MeshColorInterpolation::Smoothstep
        );

        colors.set(vec![WorkingColor::WHITE; 4]);
        points.set(vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.5, 0.5]]);
        smooths.set(false);
        let mesh = mesh(&paint);
        assert!(
            mesh.colors()
                .iter()
                .all(|color| *color == WorkingColor::WHITE)
        );
        assert_eq!(mesh.points()[3], crate::draw::kurbo::Point::new(0.5, 0.5));
        assert_eq!(mesh.interpolation_mode(), MeshColorInterpolation::Linear);
    }

    #[test]
    #[should_panic(expected = "exactly columns*rows colours")]
    fn a_short_colour_list_is_rejected() {
        let paint = MeshGradient::new(2, 2, vec![WorkingColor::BLACK; 3]).into_paint();
        let _ = paint.snapshot();
    }
}
