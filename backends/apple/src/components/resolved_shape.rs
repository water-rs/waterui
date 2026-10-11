//! The `resolved_shape` leaf: `Native<ResolvedShape>` rendered through a
//! `CAShapeLayer` — `WuiResolvedShape` + `WuiShapePath`.
//!
//! The path is rebuilt on every layout from the shape kind's normalized
//! geometry (radii clamp to half the shorter side, custom commands
//! denormalized into the bounds); the `Computed<WorkingColor>` fill is
//! watched imperatively and invalidates any enclosing captured rendering.

use cocoa_ui::path::PathBuilder;
use cocoa_ui::shape::ShapeLayer;
use waterui::graphics::color::WorkingColor;
use waterui::reactive::Signal;
use waterui::shape::{PathCommand, ResolvedShape, ShapeKind};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::HostView;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::HostView;
}

use platform::HostView;

/// A `WorkingColor` as a `CGColor` in extended linear Display-P3 — channels carried
/// straight; values above `1.0` are the color's HDR headroom already.
fn cg_color(
    color: &WorkingColor,
) -> cocoa_ui::objc2_core_foundation::CFRetained<cocoa_ui::objc2_core_graphics::CGColor> {
    let [red, green, blue, alpha] = color.components;
    cocoa_ui::color::cg_extended_linear_display_p3(
        f64::from(red),
        f64::from(green),
        f64::from(blue),
        f64::from(alpha),
    )
}

/// A normalized radius as a logical-point corner radius: `radius` times the
/// shorter side, capped at half that side — `shapePath`'s normalized arm.
fn normalized_radius(radius: f64, width: f64, height: f64) -> f64 {
    let shorter = width.min(height);
    (radius * shorter).min(shorter / 2.0)
}

/// A logical-point radius capped at half the shorter side — the fixed arm.
fn fixed_radius(radius: f64, width: f64, height: f64) -> f64 {
    let shorter = width.min(height);
    radius.min(shorter / 2.0)
}

/// The shape's `CGPath` for `bounds` — `WuiShapePath.path(in:)` mirrored
/// kind by kind; `None` for an empty bounds, as the Swift `guard` did.
fn shape_path(shape: &ResolvedShape, bounds: cocoa_ui::Rect) -> Option<PathBuilder> {
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let (w, h) = (bounds.size.width, bounds.size.height);
    let mut builder = PathBuilder::new();
    match shape.kind {
        ShapeKind::Rect => builder.rect(bounds),
        ShapeKind::Circle | ShapeKind::Ellipse => builder.ellipse_in_rect(bounds),
        ShapeKind::RoundedRect { corner_radius } => {
            builder.rounded_rect(bounds, normalized_radius(f64::from(corner_radius), w, h));
        }
        ShapeKind::FixedRoundedRect { corner_radius } => {
            builder.rounded_rect(bounds, fixed_radius(f64::from(corner_radius), w, h));
        }
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => {
            builder.uneven_rounded_rect(
                bounds,
                (
                    normalized_radius(f64::from(top_left), w, h),
                    normalized_radius(f64::from(top_right), w, h),
                    normalized_radius(f64::from(bottom_left), w, h),
                    normalized_radius(f64::from(bottom_right), w, h),
                ),
            );
        }
        ShapeKind::FixedUnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => {
            builder.uneven_rounded_rect(
                bounds,
                (
                    fixed_radius(f64::from(top_left), w, h),
                    fixed_radius(f64::from(top_right), w, h),
                    fixed_radius(f64::from(bottom_left), w, h),
                    fixed_radius(f64::from(bottom_right), w, h),
                ),
            );
        }
        ShapeKind::Capsule => builder.rounded_rect(bounds, w.min(h) / 2.0),
        ShapeKind::CustomPath => {
            for command in &shape.commands {
                apply_command(&mut builder, command, w, h, bounds);
            }
        }
    }
    Some(builder)
}

/// One normalized `PathCommand` denormalized into `bounds` —
/// `denormalizeCommands`: x scales on width, y on height, all in the bounds'
/// local space.
fn apply_command(
    builder: &mut PathBuilder,
    command: &PathCommand,
    w: f64,
    h: f64,
    bounds: cocoa_ui::Rect,
) {
    let p = |x: f32, y: f32| {
        cocoa_ui::Point::new(
            f64::from(x).mul_add(w, bounds.origin.x),
            f64::from(y).mul_add(h, bounds.origin.y),
        )
    };
    match *command {
        PathCommand::MoveTo { x, y } => builder.move_to(p(x, y)),
        PathCommand::LineTo { x, y } => builder.line_to(p(x, y)),
        PathCommand::QuadTo { cx, cy, x, y } => builder.quad_to(p(cx, cy), p(x, y)),
        PathCommand::CubicTo {
            c1x,
            c1y,
            c2x,
            c2y,
            x,
            y,
        } => builder.cubic_to(p(c1x, c1y), p(c2x, c2y), p(x, y)),
        PathCommand::Arc {
            cx,
            cy,
            rx,
            ry,
            start,
            sweep,
        } => {
            let center = p(cx, cy);
            let rx = f64::from(rx) * w;
            let ry = f64::from(ry) * h;
            builder.arc(center, rx, ry, f64::from(start), f64::from(sweep));
        }
        PathCommand::Close => builder.close(),
    }
}

/// The host view's layout face: greedy, stretching both axes.
struct ShapeSubView;

impl core::fmt::Debug for ShapeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ShapeSubView").finish_non_exhaustive()
    }
}

impl SubView for ShapeSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(Size::new(
            proposal.width.unwrap_or(0.0),
            proposal.height.unwrap_or(0.0),
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// Rebuilds the layer's frame and path for the current bounds —
/// `WuiResolvedShape.layout()`.
fn rebuild(view: &HostView, layer: &ShapeLayer, shape: &ResolvedShape) {
    let bounds = cocoa_ui::view::bounds(view);
    layer.set_frame(bounds);
    layer.set_path(
        shape_path(shape, bounds)
            .as_ref()
            .map(cocoa_ui::path::PathBuilder::build)
            .as_deref(),
    );
}

/// Installs the `resolved_shape` handler: the fill color is a `Computed`
/// watched imperatively; every change both repaints and invalidates any
/// enclosing captured rendering.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ResolvedShape>(|shape, ctx| {
        let mtm = ctx.mtm();
        let view = HostView::new(mtm, cocoa_ui::Rect::ZERO);
        #[cfg(target_os = "macos")]
        cocoa_ui::view::ensure_layer_backed(&view);
        let layer = alloc::rc::Rc::new(ShapeLayer::new());
        cocoa_ui::view::layer(&view)
            .expect("host view is layer-backed")
            .addSublayer(&layer.layer());

        rebuild(&view, &layer, &shape);
        {
            let layer = layer.clone();
            let shape = shape.clone();
            view.set_layout_handler(move |view| rebuild(view, &layer, &shape));
        }

        let mut leaf = NativeLeaf::new(&*view, ShapeSubView);
        layer.set_fill_color(Some(&cg_color(&shape.fill.snapshot())));
        leaf.bind(&shape.fill, {
            let layer = layer;
            let view = view;
            move |color| {
                layer.set_fill_color(Some(&cg_color(&color)));
                crate::invalidation::invalidate_rendered_content(&view);
            }
        });
        leaf
    });
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    #[test]
    fn normalized_radius_scales_by_shorter_side_and_clamps() {
        assert_eq!(normalized_radius(0.25, 200.0, 100.0), 25.0);
        assert_eq!(normalized_radius(0.9, 200.0, 100.0), 50.0);
    }

    #[test]
    fn fixed_radius_is_absolute_and_clamps() {
        assert_eq!(fixed_radius(28.0, 280.0, 140.0), 28.0);
        assert_eq!(fixed_radius(200.0, 280.0, 140.0), 70.0);
    }
}
