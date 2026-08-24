//! Shape views and shape clips.
//!
//! Geometry comes from [`ShapeKind`], never from the unit-space
//! [`PathCommand`] list: those coordinates are normalized per axis, so
//! resolving them against a non-square rect stretches a circular corner into
//! an elliptical one — a capsule laid out 200×40 comes out with corners
//! sweeping a tenth of its width. The kind says what the shape *is*, and a
//! normalized radius resolves against the **shorter side** as a circular
//! corner, which is how every other backend reads it. Only
//! [`ShapeKind::CustomPath`] has nothing better than the commands.
//!
//! One function, [`shape_path`], answers for both the filled shape and the
//! clip mask. A clip that disagreed with its own fill is the bug this sharing
//! forecloses.

use core::cell::RefCell;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Circle, Ellipse, PathEl, Point, Rect, RoundedRect, RoundedRectRadii};
use nami::Computed;
use waterui_core::layout::{ProposalSize, Size, StretchAxis, ViewDimensions};
use waterui_graphics::color::ResolvedColor;
use waterui_shape::{ClipShape, PathCommand, ResolvedShape, ShapeKind};

use crate::dispatch::{DewNode, DewRenderer, RenderContext, WatchedSignal};
use crate::display_list::BEZIER_TOLERANCE;
use crate::text::DewState;

/// Resolves a shape into a concrete outline for `bounds`.
///
/// `commands` is consulted only for [`ShapeKind::CustomPath`].
pub fn shape_path(kind: ShapeKind, commands: &[PathCommand], bounds: Rect) -> BezPath {
    use kurbo::Shape as _;

    let shorter = bounds.width().min(bounds.height()).max(0.0);
    let scaled = |radius: f32| f64::from(radius.clamp(0.0, 0.5)) * shorter;
    match kind {
        ShapeKind::Rect => bounds.to_path(BEZIER_TOLERANCE),
        // A circle is inscribed in the bounds: centred, its diameter the
        // shorter side. Filling the bounds instead would be an ellipse, and a
        // fully rounded rectangle would be a capsule — neither is a circle.
        ShapeKind::Circle => Circle::new(bounds.center(), shorter / 2.0).to_path(BEZIER_TOLERANCE),
        ShapeKind::Ellipse => Ellipse::from_rect(bounds).to_path(BEZIER_TOLERANCE),
        ShapeKind::RoundedRect { corner_radius } => {
            RoundedRect::from_rect(bounds, scaled(corner_radius)).to_path(BEZIER_TOLERANCE)
        }
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => RoundedRect::from_rect(
            bounds,
            RoundedRectRadii::new(
                scaled(top_left),
                scaled(top_right),
                scaled(bottom_right),
                scaled(bottom_left),
            ),
        )
        .to_path(BEZIER_TOLERANCE),
        ShapeKind::Capsule => {
            RoundedRect::from_rect(bounds, shorter / 2.0).to_path(BEZIER_TOLERANCE)
        }
        ShapeKind::CustomPath => custom_path(commands, bounds),
    }
}

/// Traces the unit-space command list against `bounds`.
///
/// Each coordinate scales with its own axis, which is exactly what a custom
/// path asks for: the author drew it in the unit square and expects it to
/// stretch with the view.
fn custom_path(commands: &[PathCommand], bounds: Rect) -> BezPath {
    let width = bounds.width();
    let height = bounds.height();
    let point = |x: f32, y: f32| {
        Point::new(
            f64::from(x).mul_add(width, bounds.x0),
            f64::from(y).mul_add(height, bounds.y0),
        )
    };
    let mut path = BezPath::new();
    for command in commands {
        match *command {
            PathCommand::MoveTo { x, y } => path.move_to(point(x, y)),
            PathCommand::LineTo { x, y } => path.line_to(point(x, y)),
            PathCommand::QuadTo { cx, cy, x, y } => path.quad_to(point(cx, cy), point(x, y)),
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => path.curve_to(point(c1x, c1y), point(c2x, c2y), point(x, y)),
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => append_arc(
                &mut path,
                point(cx, cy),
                rx,
                ry,
                start,
                sweep,
                width,
                height,
            ),
            PathCommand::Close => path.close_path(),
        }
    }
    path
}

/// Appends an elliptical arc, connecting it to the current point the way a
/// path builder with an implicit line does.
#[expect(
    clippy::too_many_arguments,
    reason = "one arc command carries exactly these parameters"
)]
fn append_arc(
    path: &mut BezPath,
    center: Point,
    radius_x: f32,
    radius_y: f32,
    start: f32,
    sweep: f32,
    width: f64,
    height: f64,
) {
    let radii = (f64::from(radius_x) * width, f64::from(radius_y) * height);
    let arc = kurbo::Arc::new(center, radii, f64::from(start), f64::from(sweep), 0.0);
    let mut segments = arc.append_iter(BEZIER_TOLERANCE);
    // The first element is a `MoveTo` to the arc's start; a path already under
    // construction wants a line there instead, and an empty one wants the move.
    if let Some(PathEl::MoveTo(entry)) = segments.next() {
        if path.elements().is_empty() {
            path.move_to(entry);
        } else {
            path.line_to(entry);
        }
    }
    path.extend(segments);
}

/// The retained node behind [`ResolvedShape`].
struct ShapeNode {
    kind: ShapeKind,
    commands: Vec<PathCommand>,
    fill: WatchedSignal<Computed<ResolvedColor>>,
}

impl DewNode for ShapeNode {
    fn measure(&self, _state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        // A shape has no intrinsic size: like a colour, it takes what it is
        // offered and is sized by `.frame()` or its container.
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        let path = shape_path(self.kind, &self.commands, ctx.bounds);
        let color = self.fill.get();
        let srgb = color.to_srgb_with_headroom();
        renderer
            .list_mut()
            .push(crate::display_list::DrawCommand::FillPath {
                path,
                transform: ctx.transform,
                brush: peniko::Color::new([srgb.red, srgb.green, srgb.blue, color.opacity]).into(),
                clip: None,
            });
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }
}

/// Builds the retained node for a resolved shape view.
pub fn build(renderer: &DewRenderer, shape: ResolvedShape) -> Box<dyn DewNode> {
    Box::new(ShapeNode {
        kind: shape.kind,
        commands: shape.commands,
        fill: WatchedSignal::new(shape.fill, renderer.signals()),
    })
}

/// A window-space mask, retained across frames.
///
/// The display list diffs a clip by pointer, so rebuilding the path every
/// frame would make every command under it compare unequal and dirty the whole
/// clipped subtree forever. The mask therefore survives until the placement
/// that produced it changes.
struct RetainedMask {
    transform: Affine,
    bounds: Rect,
    path: Arc<BezPath>,
}

/// The retained node behind `.clip(shape)`.
struct ClipNode {
    shape: ClipShape,
    child: Box<dyn DewNode>,
    mask: RefCell<Option<RetainedMask>>,
}

impl ClipNode {
    /// The window-space mask for `ctx`, rebuilt only when the placement moves.
    fn mask(&self, ctx: RenderContext) -> Arc<BezPath> {
        let mut cache = self.mask.borrow_mut();
        if let Some(mask) = cache.as_ref()
            && mask.transform == ctx.transform
            && mask.bounds == ctx.bounds
        {
            return Arc::clone(&mask.path);
        }
        let mut path = shape_path(self.shape.kind(), self.shape.commands(), ctx.bounds);
        path.apply_affine(ctx.transform);
        let path = Arc::new(path);
        *cache = Some(RetainedMask {
            transform: ctx.transform,
            bounds: ctx.bounds,
            path: Arc::clone(&path),
        });
        path
    }
}

impl DewNode for ClipNode {
    fn measure(&self, state: &RefCell<DewState>, proposal: ProposalSize) -> ViewDimensions {
        self.child.measure(state, proposal)
    }

    fn render(&mut self, renderer: &mut DewRenderer, ctx: RenderContext) {
        // A rectangular mask is exactly the rectangle arm the scroll viewport
        // already uses, and dew places views by translation alone, so the
        // transformed box is the mask rather than an approximation of it.
        if matches!(self.shape.kind(), ShapeKind::Rect) {
            renderer
                .list_mut()
                .push_clip(ctx.transform.transform_rect_bbox(ctx.bounds));
        } else {
            let mask = self.mask(ctx);
            renderer.list_mut().push_clip_shape(mask);
        }
        self.child.render(renderer, ctx);
        renderer.list_mut().pop_clip();
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.child.stretch_axis()
    }

    fn patch(&mut self, renderer: &mut DewRenderer) -> bool {
        self.child.patch(renderer)
    }
}

/// Builds the retained node for a clipped subtree.
pub fn build_clip(shape: ClipShape, child: Box<dyn DewNode>) -> Box<dyn DewNode> {
    Box::new(ClipNode {
        shape,
        child,
        mask: RefCell::new(None),
    })
}
