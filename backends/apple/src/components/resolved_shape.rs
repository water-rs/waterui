//! The `resolved_shape` leaf: `Native<ResolvedShape>` rendered through a
//! `CAShapeLayer` for solid fills or a `CAShapeLayer`-masked `GradientLayer`
//! for gradient fills.
//!
//! The path is rebuilt on every layout from the shape kind's normalized
//! geometry (radii clamp to half the shorter side, custom commands
//! denormalized into the bounds); the `Computed<Paint>` fill is watched
//! imperatively and invalidates any enclosing captured rendering.

use alloc::rc::Rc;
use core::cell::RefCell;

use cocoa_ui::MainThreadMarker;
use cocoa_ui::gradient::GradientLayer;
use cocoa_ui::path::PathBuilder;
use cocoa_ui::shape::{ShapeLayer, set_layer_mask};
use nami::Signal;
use waterui_core::layout::{
    LayoutError, ProposalSize, Rect, ResolvedExtent, Size, StretchAxis, SubView, ViewDimensions,
};
use waterui_graphics::backend::draw::Paint;
use waterui_graphics::color::WorkingColor;
use waterui_shape::{PathCommand, ResolvedShape, ShapeKind};

use super::resolved_gradient::{layer_paint, update_contents_scale};
use crate::contract::{NativeLeaf, RenderContext};

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

/// Resolves a `CornerRadius` into logical points capped at half the shorter side.
pub fn resolve_corner(radius: waterui_shape::CornerRadius, shorter: f64) -> f64 {
    let limit = shorter / 2.0;
    match (radius.as_points(), radius.as_fraction()) {
        (Some(points), None) => f64::from(points).min(limit),
        (None, Some(fraction)) => (f64::from(fraction) * shorter).min(limit),
        _ => unreachable!("CornerRadius is either points or fraction"),
    }
}

/// The shape's `CGPath` for `bounds` — `WuiShapePath.path(in:)` mirrored
/// kind by kind; `None` for an empty bounds, as the Swift `guard` did.
pub fn shape_path(
    kind: ShapeKind,
    commands: &[PathCommand],
    bounds: cocoa_ui::Rect,
) -> Option<PathBuilder> {
    if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
        return None;
    }
    let (w, h) = (bounds.size.width, bounds.size.height);
    let shorter = w.min(h);
    let mut builder = PathBuilder::new();
    match kind {
        ShapeKind::Rect => builder.rect(bounds),
        ShapeKind::Circle => {
            let diameter = shorter;
            let square = cocoa_ui::Rect::new(
                bounds.origin.x + (w - diameter) / 2.0,
                bounds.origin.y + (h - diameter) / 2.0,
                diameter,
                diameter,
            );
            builder.ellipse_in_rect(square);
        }
        ShapeKind::Ellipse => builder.ellipse_in_rect(bounds),
        ShapeKind::RoundedRect { radii } => {
            if let Some(uniform) = radii.as_uniform() {
                builder.rounded_rect(bounds, resolve_corner(uniform, shorter));
            } else {
                builder.uneven_rounded_rect(
                    bounds,
                    (
                        resolve_corner(radii.top_leading(), shorter),
                        resolve_corner(radii.top_trailing(), shorter),
                        resolve_corner(radii.bottom_leading(), shorter),
                        resolve_corner(radii.bottom_trailing(), shorter),
                    ),
                );
            }
        }
        ShapeKind::Capsule => builder.rounded_rect(bounds, shorter / 2.0),
        ShapeKind::CustomPath => {
            for command in commands {
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
            builder.arc(
                center,
                rx,
                ry,
                f64::from(start.to_radians()),
                f64::from(sweep.to_radians()),
            );
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
    fn resolve_extent(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
    ) -> Result<ResolvedExtent, LayoutError> {
        ResolvedExtent::centred(bounds, self.measure(proposal).size)
    }

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

enum FillPresentation {
    Solid(ShapeLayer),
    Gradient {
        gradient: GradientLayer,
        mask: ShapeLayer,
    },
}

impl FillPresentation {
    fn new(mtm: MainThreadMarker, view: &HostView, shape: &ResolvedShape, paint: &Paint) -> Self {
        let host_layer = cocoa_ui::view::layer(view).expect("host view is layer-backed");
        let presentation = match paint {
            Paint::Solid(color) => {
                let layer = ShapeLayer::new();
                layer.set_fill_color(Some(&cg_color(color)));
                host_layer.addSublayer(&layer.layer());
                Self::Solid(layer)
            }
            gradient_paint => {
                let gradient = GradientLayer::new(mtm, layer_paint(gradient_paint));
                let mask = ShapeLayer::new();
                mask.set_fill_color(Some(&cg_color(&WorkingColor::WHITE)));
                set_layer_mask(&gradient.layer(), Some(&mask.layer()));
                host_layer.addSublayer(&gradient.layer());
                Self::Gradient { gradient, mask }
            }
        };
        presentation.rebuild(view, shape);
        presentation
    }

    fn remove_from_superlayer(&self) {
        match self {
            Self::Solid(layer) => layer.layer().removeFromSuperlayer(),
            Self::Gradient { gradient, .. } => gradient.layer().removeFromSuperlayer(),
        }
    }

    fn rebuild(&self, view: &HostView, shape: &ResolvedShape) {
        let bounds = cocoa_ui::view::bounds(view);
        let built = shape_path(shape.kind(), shape.commands(), bounds)
            .as_ref()
            .map(PathBuilder::build);
        match self {
            Self::Solid(layer) => {
                layer.set_frame(bounds);
                layer.set_path(built.as_deref());
            }
            Self::Gradient { gradient, mask } => {
                gradient.set_frame(bounds);
                update_contents_scale(view, gradient);
                mask.set_frame(bounds);
                mask.set_path(built.as_deref());
            }
        }
    }

    fn update_scale(&self, view: &HostView) {
        if let Self::Gradient { gradient, .. } = self {
            update_contents_scale(view, gradient);
        }
    }

    fn update_paint(
        &mut self,
        mtm: MainThreadMarker,
        view: &HostView,
        shape: &ResolvedShape,
        paint: &Paint,
    ) {
        match (self, paint) {
            (Self::Solid(layer), Paint::Solid(color)) => {
                layer.set_fill_color(Some(&cg_color(color)));
            }
            (Self::Gradient { gradient, .. }, gradient_paint)
                if !matches!(gradient_paint, Paint::Solid(_)) =>
            {
                gradient.set_paint(layer_paint(gradient_paint));
            }
            (slot, next_paint) => {
                slot.remove_from_superlayer();
                *slot = Self::new(mtm, view, shape, next_paint);
            }
        }
    }
}

/// The `resolved_shape` realization: the fill paint is a `Computed<Paint>`
/// watched imperatively; every change both repaints and invalidates any
/// enclosing captured rendering.
#[expect(
    clippy::needless_pass_by_value,
    reason = "NativeBackend hands each payload by value"
)]
pub fn realize_resolved_shape(shape: ResolvedShape, ctx: &RenderContext<'_>) -> NativeLeaf {
    let mtm = ctx.mtm();
    let view = HostView::new(mtm, cocoa_ui::Rect::ZERO);
    #[cfg(target_os = "macos")]
    cocoa_ui::view::ensure_layer_backed(&view);

    let initial = shape.fill().snapshot();
    let presentation = Rc::new(RefCell::new(FillPresentation::new(
        mtm, &view, &shape, &initial,
    )));

    {
        let presentation = Rc::clone(&presentation);
        let shape = shape.clone();
        view.set_layout_handler(move |view| presentation.borrow().rebuild(view, &shape));
    }
    {
        let presentation = Rc::clone(&presentation);
        view.set_window_handler(move |view| presentation.borrow().update_scale(view));
    }
    {
        let presentation = Rc::clone(&presentation);
        view.set_backing_changed_handler(move |view| presentation.borrow().update_scale(view));
    }

    let mut leaf = NativeLeaf::new(&*view, ShapeSubView);
    leaf.bind(shape.fill(), {
        let presentation = presentation;
        let view = view;
        let shape = shape.clone();
        move |paint| {
            presentation
                .borrow_mut()
                .update_paint(mtm, &view, &shape, &paint);
            crate::invalidation::invalidate_rendered_content(&view);
        }
    });
    leaf
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::resolve_corner;
    use waterui_shape::CornerRadius;

    #[test]
    fn fraction_radius_scales_by_shorter_side_and_clamps() {
        assert_eq!(resolve_corner(CornerRadius::fraction(0.25), 100.0), 25.0);
        assert_eq!(resolve_corner(CornerRadius::fraction(0.9), 100.0), 50.0);
    }

    #[test]
    fn point_radius_is_absolute_and_clamps() {
        assert_eq!(resolve_corner(CornerRadius::points(28.0), 140.0), 28.0);
        assert_eq!(resolve_corner(CornerRadius::points(200.0), 140.0), 70.0);
    }
}
