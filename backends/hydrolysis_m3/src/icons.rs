//! Material icon views drawn straight into the scene.
//!
//! These icons are stroked outlines rather than glyphs, so they are authored as
//! [`SceneContent`]: the icon owns its path and re-strokes it whenever the
//! colour token it resolved against changes, which is the precise update the
//! reactive contract asks for.

use core::cell::Cell;
use std::rc::Rc;

use vello::kurbo::{Affine, BezPath, Cap, Join, Point, Stroke};
use vello::peniko::{Brush, Color};
use waterui::color::ResolvedColor;
use waterui::{Environment, View, ViewExt as _};
use waterui_core::Signal;
use waterui_core::resolve::Resolvable;
use waterui_graphics::{Scene2D, SceneContent, SceneInvalidator, SceneView};

/// The miter limit Material icon strokes are authored against.
const MITER_LIMIT: f64 = 10.0;

/// A shared Material checkmark, drawn as a stroked polyline.
#[derive(Debug, Clone, Copy)]
pub struct CheckmarkIcon<ColorToken> {
    color: ColorToken,
    size: f32,
    line_width: f32,
    container_height: f32,
}

impl<ColorToken> CheckmarkIcon<ColorToken> {
    pub(crate) const fn new(color: ColorToken, size: f32, line_width: f32) -> Self {
        Self {
            color,
            size,
            line_width,
            container_height: size,
        }
    }

    pub(crate) const fn container_height(mut self, container_height: f32) -> Self {
        self.container_height = container_height;
        self
    }
}

impl<ColorToken> View for CheckmarkIcon<ColorToken>
where
    ColorToken: Resolvable<Resolved = ResolvedColor> + 'static,
{
    fn body(self, env: &Environment) -> impl View {
        SceneView::new(CheckmarkContent {
            stroke: self.color.resolve(env),
            size: self.size,
            line_width: self.line_width,
            container_height: self.container_height,
            invalidator: None,
            pending_redraw: Rc::new(Cell::new(false)),
            guard: None,
        })
        .size(self.size, self.size)
    }
}

/// The checkmark's scene content: one stroked polyline whose colour follows a
/// signal, re-encoded when that signal fires.
struct CheckmarkContent<S: Signal> {
    stroke: S,
    size: f32,
    line_width: f32,
    container_height: f32,
    invalidator: Option<SceneInvalidator>,
    pending_redraw: Rc<Cell<bool>>,
    guard: Option<S::Guard>,
}

impl<S> SceneContent for CheckmarkContent<S>
where
    S: Signal<Output = ResolvedColor> + 'static,
    S::Guard: 'static,
{
    fn build_scene(&mut self, scene: &mut dyn Scene2D, width: f32, _height: f32) -> bool {
        self.pending_redraw.set(false);
        let pending_redraw = Rc::clone(&self.pending_redraw);
        let invalidator = self.invalidator.clone();
        self.guard = Some(self.stroke.watch(move |_| {
            pending_redraw.set(true);
            if let Some(invalidator) = &invalidator {
                invalidator();
            }
        }));

        let x = (width - self.size) * 0.5;
        let y = (self.container_height - self.size) * 0.5;
        let mut path = BezPath::new();
        path.move_to(point(x + 4.25, y + 9.25));
        path.line_to(point(x + 7.25, y + 12.25));
        path.line_to(point(x + 14.0, y + 5.5));

        let stroke = Stroke::new(f64::from(self.line_width))
            .with_caps(Cap::Round)
            .with_join(Join::Round)
            .with_miter_limit(MITER_LIMIT);
        let brush = Brush::Solid(to_peniko(self.stroke.get()));
        scene.stroke(&stroke, Affine::IDENTITY, &brush, None, &path);

        self.pending_redraw.replace(false)
    }

    fn set_invalidator(&mut self, invalidator: Option<SceneInvalidator>) {
        self.invalidator = invalidator;
    }
}

/// Widens an icon-space coordinate pair, authored in `f32`, into the `f64` the
/// path builder takes.
fn point(x: f32, y: f32) -> Point {
    Point::new(f64::from(x), f64::from(y))
}

/// Converts a resolved theme colour into the scene's paint.
fn to_peniko(color: ResolvedColor) -> Color {
    let srgb = color.to_srgb_with_headroom();
    Color::new([
        srgb.red,
        srgb.green,
        srgb.blue,
        color.opacity.clamp(0.0, 1.0),
    ])
}
