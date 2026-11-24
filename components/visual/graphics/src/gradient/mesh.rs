//! A mesh gradient whose colours and control points follow signals.

extern crate alloc;

use alloc::vec::Vec;
use core::fmt;

use crate::draw::kurbo::Rect;
use crate::draw::{Draw as _, Paint, Recorder};
use nami::map::map;
use nami::{Computed, Signal, SignalExt};
use waterui_color::Color;
use waterui_core::layout::UnitPoint;
use waterui_core::reactive::signal::IntoComputed;
use waterui_core::{Environment, View, contract_violation};

use super::paint::{Gradient, mesh_paint, resolve_colors};
use crate::scene::resources::RecordingResources;
use crate::scene_view::{NextFrame, SceneContent, SceneView};

/// A mesh gradient over a `columns` × `rows` grid of vertices whose colours,
/// control points and interpolation follow signals.
///
/// The vertices are row-major in unit space, `(0, 0)` at the top-left. Until
/// [`points`](Self::points) says otherwise they sit on an even grid, the
/// first at the top-left corner and the last at the bottom-right.
///
/// Vertex colours resolve in the view's [`Environment`], and any change to
/// the colour scheme or to a signal replaces the gradient's paint in the
/// recorded scene in place without rebuilding the view.
///
/// # Layout Behavior
///
/// A mesh gradient is a greedy view: it expands to fill all available space
/// on both axes. Constrain it with `.frame()` or use it as a background.
pub struct MeshGradient {
    columns: u32,
    rows: u32,
    colors: Computed<Vec<Color>>,
    points: Computed<Vec<UnitPoint>>,
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
    /// A grid with fewer than two vertices on a side is a contract
    /// violation: the grid has two vertices on that side.
    #[must_use]
    pub fn new<C>(columns: u32, rows: u32, colors: C) -> Self
    where
        C: Signal + 'static,
        C::Output: IntoIterator<Item = Color>,
    {
        let (columns, rows) = if columns >= 2 && rows >= 2 {
            (columns, rows)
        } else {
            contract_violation!(
                "mesh gradients need at least a 2x2 grid of vertices, got {columns}x{rows}; a side with fewer than two vertices has two"
            );
            (columns.max(2), rows.max(2))
        };
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
        P::Output: IntoIterator<Item = UnitPoint>,
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

    /// Resolves this mesh's vertex colours and control points in `env` into a
    /// live [`Paint`] in unit space.
    #[must_use]
    pub fn resolve(&self, env: &Environment) -> Computed<Paint> {
        let columns = self.columns;
        let rows = self.rows;
        let colors = resolve_colors(&self.colors, env);
        colors
            .zip(&self.points)
            .zip(&self.smooths_colors)
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
fn even_grid(columns: u32, rows: u32) -> Vec<UnitPoint> {
    (0..rows)
        .flat_map(|row| {
            (0..columns).map(move |column| {
                UnitPoint::new(grid_fraction(column, columns), grid_fraction(row, rows))
            })
        })
        .collect()
}

impl View for MeshGradient {
    fn body(self, env: &Environment) -> impl View {
        SceneView::new(MeshContent {
            paint: self.resolve(env),
        })
    }
}

/// Fills the view with the live mesh paint, authored in unit space.
struct MeshContent {
    paint: Computed<Paint>,
}

impl SceneContent for MeshContent {
    fn build_scene(
        &mut self,
        recorder: &mut Recorder,
        _resources: &mut RecordingResources<'_>,
        width: f32,
        height: f32,
    ) -> NextFrame {
        let paint = self.paint.clone();
        recorder.transform(Gradient::transform_to(width, height), |recorder| {
            recorder.fill(Rect::new(0.0, 0.0, 1.0, 1.0), paint);
        });
        NextFrame::Idle
    }

    fn rebuild_for_engine(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{MeshColorInterpolation, WorkingColor};
    use nami::Binding;
    use waterui_color::{ColorScheme, Working};

    fn working(color: WorkingColor) -> Color {
        Color::new(Working(color))
    }

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
                UnitPoint::new(0.0, 0.0),
                UnitPoint::new(0.5, 0.0),
                UnitPoint::new(1.0, 0.0),
                UnitPoint::new(0.0, 1.0),
                UnitPoint::new(0.5, 1.0),
                UnitPoint::new(1.0, 1.0),
            ]
        );
    }

    #[test]
    fn the_paint_follows_its_signals() {
        let colors = Binding::container(vec![working(WorkingColor::BLACK); 4]);
        let points = Binding::container(even_grid(2, 2));
        let smooths = Binding::container(true);
        let paint = MeshGradient::new(2, 2, colors.clone())
            .points(points.clone())
            .smooths_colors(smooths.clone())
            .resolve(&Environment::new());
        assert_eq!(
            mesh(&paint).interpolation_mode(),
            MeshColorInterpolation::Smoothstep
        );

        colors.set(vec![working(WorkingColor::WHITE); 4]);
        points.set(vec![
            UnitPoint::TOP_LEADING,
            UnitPoint::TOP_TRAILING,
            UnitPoint::BOTTOM_LEADING,
            UnitPoint::CENTER,
        ]);
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
    fn theme_token_mesh_recolours_on_scheme_change() {
        use waterui::theme::color::{Accent, Surface};

        let scheme = Binding::container(ColorScheme::Light);
        let mut env = Environment::new();
        waterui::theme::install_color_scheme(&mut env, scheme.computed());
        waterui::theme::install_color_signal::<Accent>(
            &mut env,
            scheme
                .map(|mode| match mode {
                    ColorScheme::Light => WorkingColor::WHITE,
                    ColorScheme::Dark => WorkingColor::BLACK,
                })
                .computed(),
        );
        waterui::theme::install_color_signal::<Surface>(
            &mut env,
            scheme
                .map(|mode| match mode {
                    ColorScheme::Light => WorkingColor::BLACK,
                    ColorScheme::Dark => WorkingColor::WHITE,
                })
                .computed(),
        );

        let paint = MeshGradient::new(
            2,
            2,
            [Accent.into(), Surface.into(), Surface.into(), Accent.into()],
        )
        .resolve(&env);
        assert_eq!(mesh(&paint).colors()[0], WorkingColor::WHITE);
        assert_eq!(mesh(&paint).colors()[1], WorkingColor::BLACK);

        scheme.set(ColorScheme::Dark);

        assert_eq!(mesh(&paint).colors()[0], WorkingColor::BLACK);
        assert_eq!(mesh(&paint).colors()[1], WorkingColor::WHITE);
    }

    #[test]
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "exactly columns*rows colours")
    )]
    fn a_short_colour_list_is_a_transparent_patch() {
        let paint = MeshGradient::new(2, 2, vec![working(WorkingColor::BLACK); 3])
            .resolve(&Environment::new());
        let mesh = mesh(&paint);
        assert_eq!((mesh.columns(), mesh.rows()), (1, 1));
        assert!(
            mesh.colors()
                .iter()
                .all(|color| *color == WorkingColor::TRANSPARENT)
        );
    }

    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "at least a 2x2 grid"))]
    fn a_one_column_grid_has_two_columns() {
        let paint = MeshGradient::new(1, 2, vec![working(WorkingColor::BLACK); 4])
            .resolve(&Environment::new());
        assert_eq!(mesh(&paint).points().len(), 4);
    }
}
