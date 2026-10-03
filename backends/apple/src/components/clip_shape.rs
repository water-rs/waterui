//! The `clip_shape` metadata: `Metadata<ClipShape>` wrapped around content
//! the bounds clip to a shape.
//!
//! Mirrors `WuiClipShape`: a transparent `HostView` container — measure,
//! stretch, priority and the placement proposal all answer for the mounted
//! child — whose backing layer carries a `CAShapeLayer` mask rebuilt on
//! every layout pass. `UIKit` clips through `clipsToBounds`, `AppKit`
//! through `masksToBounds` on the backing layer.

use alloc::rc::Rc;
use alloc::vec::Vec;
use core::cell::RefCell;

use cocoa_ui::shape::{self, PathBuilder};
use cocoa_ui::view;
use cocoa_ui::{PlatformView, Point, Rect, Retained};
use waterui::shape::{ClipShape, PathCommand, ShapeKind};
use waterui_core::Metadata;
use waterui_core::layout::{ProposalSize, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf};
use crate::dispatch::Dispatcher;
use crate::proposal;

use objc2_core_foundation::CFRetained;
use objc2_core_graphics::CGPath;
use objc2_quartz_core::CAShapeLayer;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// Scales a normalized shape-space coordinate into `bounds` on its axis.
fn denormalize(x: f32, y: f32, bounds: Rect) -> Point {
    Point::new(
        f64::from(x) * bounds.size.width,
        f64::from(y) * bounds.size.height,
    )
}

/// Builds the unit-space command path the shape was authored with.
fn commands_path(commands: &[PathCommand], bounds: Rect) -> CFRetained<CGPath> {
    let builder = PathBuilder::new();
    for command in commands {
        match *command {
            PathCommand::MoveTo { x, y } => {
                builder.move_to(denormalize(x, y, bounds));
            }
            PathCommand::LineTo { x, y } => {
                builder.line_to(denormalize(x, y, bounds));
            }
            PathCommand::QuadTo { cx, cy, x, y } => {
                builder.quad_to(denormalize(cx, cy, bounds), denormalize(x, y, bounds));
            }
            PathCommand::CubicTo {
                c1x,
                c1y,
                c2x,
                c2y,
                x,
                y,
            } => {
                builder.cubic_to(
                    denormalize(c1x, c1y, bounds),
                    denormalize(c2x, c2y, bounds),
                    denormalize(x, y, bounds),
                );
            }
            PathCommand::Arc {
                cx,
                cy,
                rx,
                ry,
                start,
                sweep,
            } => {
                builder.arc(
                    denormalize(cx, cy, bounds),
                    Point::new(
                        f64::from(rx) * bounds.size.width,
                        f64::from(ry) * bounds.size.height,
                    ),
                    f64::from(start),
                    f64::from(sweep),
                );
            }
            PathCommand::Close => {
                builder.close();
            }
        }
    }
    builder.finish()
}

/// The per-corner-arc rounded rect `ShapeKind` spells without a single
/// `roundedRect` call.
fn uneven_rounded_rect(
    tl: f32,
    tr: f32,
    br: f32,
    bl: f32,
    scale: f64,
    limit: f64,
    bounds: Rect,
) -> CFRetained<CGPath> {
    let tl = (f64::from(tl) * scale).min(limit);
    let tr = (f64::from(tr) * scale).min(limit);
    let br = (f64::from(br) * scale).min(limit);
    let bl = (f64::from(bl) * scale).min(limit);

    let min_x = bounds.origin.x;
    let min_y = bounds.origin.y;
    let max_x = bounds.origin.x + bounds.size.width;
    let max_y = bounds.origin.y + bounds.size.height;

    let builder = PathBuilder::new();
    builder.move_to(Point::new(min_x + tl, min_y));
    builder.arc_to_point(Point::new(max_x, min_y), Point::new(max_x, max_y), tr);
    builder.arc_to_point(Point::new(max_x, max_y), Point::new(min_x, max_y), br);
    builder.arc_to_point(Point::new(min_x, max_y), Point::new(min_x, min_y), bl);
    builder.arc_to_point(Point::new(min_x, min_y), Point::new(max_x, min_y), tl);
    builder.close();
    builder.finish()
}

/// Resolves the shape into a `CGPath` in `bounds`, preferring the
/// structured kind over the unit-space commands so a normalized radius
/// still resolves against the shorter side.
fn shape_path(kind: ShapeKind, commands: &[PathCommand], bounds: Rect) -> CFRetained<CGPath> {
    let shorter = bounds.size.width.min(bounds.size.height);
    match kind {
        ShapeKind::Rect => shape::rect_path(bounds),
        ShapeKind::Circle => {
            let diameter = shorter;
            shape::ellipse_path(Rect::new(
                bounds.origin.x + (bounds.size.width - diameter) / 2.0,
                bounds.origin.y + (bounds.size.height - diameter) / 2.0,
                diameter,
                diameter,
            ))
        }
        ShapeKind::Ellipse => shape::ellipse_path(bounds),
        ShapeKind::RoundedRect { corner_radius } => {
            let radius = (f64::from(corner_radius) * shorter).min(shorter / 2.0);
            shape::rounded_rect_path(bounds, radius)
        }
        ShapeKind::UnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => uneven_rounded_rect(
            top_left,
            top_right,
            bottom_right,
            bottom_left,
            shorter,
            shorter / 2.0,
            bounds,
        ),
        ShapeKind::Capsule => shape::rounded_rect_path(bounds, shorter / 2.0),
        ShapeKind::CustomPath => commands_path(commands, bounds),
        ShapeKind::FixedRoundedRect { corner_radius } => {
            let radius = f64::from(corner_radius).min(shorter / 2.0);
            shape::rounded_rect_path(bounds, radius)
        }
        ShapeKind::FixedUnevenRoundedRect {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
        } => uneven_rounded_rect(
            top_left,
            top_right,
            bottom_right,
            bottom_left,
            1.0,
            shorter / 2.0,
            bounds,
        ),
    }
}

/// The leaf's live state: the mounted child the layout face forwards to,
/// plus the mask layer rebuilt each layout pass.
struct ClipShapeState {
    /// The mounted content.
    child: Mounted,
    /// The `CAShapeLayer` mask, created on the first non-empty layout.
    mask: RefCell<Option<Retained<CAShapeLayer>>>,
    /// The structured shape kind.
    kind: ShapeKind,
    /// The unit-space path commands, used when `kind` is `CustomPath`.
    commands: Vec<PathCommand>,
}

impl core::fmt::Debug for ClipShapeState {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClipShapeState").finish_non_exhaustive()
    }
}

/// The wrapper's layout face: transparent — every answer the child's.
struct ClipShapeSubView {
    /// The leaf's state.
    state: Rc<ClipShapeState>,
}

impl core::fmt::Debug for ClipShapeSubView {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ClipShapeSubView").finish_non_exhaustive()
    }
}

impl SubView for ClipShapeSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        self.state.child.layout().measure(proposal)
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.state.child.layout().stretch_axis()
    }

    fn priority(&self) -> i32 {
        self.state.child.layout().priority()
    }

    fn is_empty(&self) -> bool {
        self.state.child.layout().is_empty()
    }
}

/// Installs the `clip_shape` handler on the dispatcher:
/// `Metadata<ClipShape>` maps to a transparent container whose backing
/// layer masks its bounds to the shape.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<ClipShape>>(|metadata, ctx| {
        let mtm = ctx.mtm();
        let host = HostView::new(mtm, Rect::ZERO);
        let host_view: &PlatformView = &host;

        let mounted = ctx.render(metadata.content).mount(host_view);
        crate::primary_content::forward(&host, mounted.view());
        view::set_translates_autoresizing(mounted.view(), true);

        // `UIKit` clips through the view; `AppKit` through the backing
        // layer `setWantsLayer` installs.
        #[cfg(target_os = "ios")]
        host_view.setClipsToBounds(true);
        #[cfg(target_os = "macos")]
        if let Some(layer) = shape::layer(host_view) {
            layer.setMasksToBounds(true);
        }

        let state = Rc::new(ClipShapeState {
            child: mounted,
            mask: RefCell::new(None),
            kind: metadata.value.kind(),
            commands: metadata.value.commands().to_vec(),
        });

        // Each layout pass stretches the content over the bounds and
        // rebuilds the mask at the new size, as `WuiClipShape.updateMask`.
        host.set_layout_handler({
            let state = Rc::clone(&state);
            let host_view = Retained::from(host_view);
            move |host| {
                let bounds = view::bounds(host);
                view::set_frame(state.child.view(), bounds);
                if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
                    return;
                }
                {
                    let mut mask = state.mask.borrow_mut();
                    if mask.is_none() {
                        let layer = shape::shape_layer();
                        shape::set_mask(&host_view, Some(&layer));
                        *mask = Some(layer);
                    }
                    if let Some(mask) = mask.as_ref() {
                        let path = shape_path(state.kind, &state.commands, bounds);
                        shape::set_path(mask, &path);
                    }
                }
            }
        });

        // `setPlacementProposal`: the proposal selected for this wrapper is
        // the proposal its content was negotiated with.
        let sink_guard = proposal::register_sink(host_view, {
            let state = Rc::clone(&state);
            move |selected| {
                proposal::deliver(state.child.view(), selected);
            }
        });

        let mut leaf = NativeLeaf::new(
            host_view,
            ClipShapeSubView {
                state: Rc::clone(&state),
            },
        );
        leaf.keep(sink_guard);
        leaf.keep(state);
        leaf
    });
}

#[cfg(test)]
mod tests {
    use alloc::vec;
    use alloc::vec::Vec;

    use super::*;

    /// Produces the unit-space path a `ClipShape` carries and resolves it
    /// into `bounds`; the path is inspected through
    /// `CGPath::contains_point`.
    #[test]
    fn rect_kind_fills_bounds() {
        let bounds = Rect::new(0.0, 0.0, 100.0, 50.0);
        let path = shape_path(ShapeKind::Rect, &[], bounds);
        // A point inside the bounds is inside the path.
        let inside = objc2_core_foundation::CGPoint::new(50.0, 25.0);
        // SAFETY: `path` is a live path built above; null transform.
        assert!(unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), inside, false) });
        let outside = objc2_core_foundation::CGPoint::new(120.0, 25.0);
        // SAFETY: same.
        assert!(!unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), outside, false) });
    }

    #[test]
    fn circle_is_centered_on_shorter_side() {
        // 100x50 bounds: the circle's diameter is 50, centered.
        let bounds = Rect::new(0.0, 0.0, 100.0, 50.0);
        let path = shape_path(ShapeKind::Circle, &[], bounds);
        let center = objc2_core_foundation::CGPoint::new(50.0, 25.0);
        // SAFETY: `path` is a live path built above; null transform.
        assert!(unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), center, false) });
        // A corner point outside the inscribed circle but inside bounds.
        let corner = objc2_core_foundation::CGPoint::new(50.0 + 24.0, 25.0 + 24.0);
        // SAFETY: same.
        assert!(!unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), corner, false) });
    }

    #[test]
    fn capsule_rounds_shorter_side() {
        // A capsule in a wide rect: the top-center point is clipped by the
        // rounded corner while the midpoint stays inside.
        let bounds = Rect::new(0.0, 0.0, 200.0, 40.0);
        let path = shape_path(ShapeKind::Capsule, &[], bounds);
        let middle = objc2_core_foundation::CGPoint::new(100.0, 20.0);
        // SAFETY: `path` is a live path built above; null transform.
        assert!(unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), middle, false) });
        // (5, 5) is inside the bounding rect but outside the capsule's arc.
        let clipped = objc2_core_foundation::CGPoint::new(5.0, 5.0);
        // SAFETY: same.
        assert!(!unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), clipped, false) });
    }

    #[test]
    fn custom_path_uses_commands() {
        let commands: Vec<PathCommand> = vec![
            PathCommand::MoveTo { x: 0.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 0.0 },
            PathCommand::LineTo { x: 1.0, y: 1.0 },
            PathCommand::Close,
        ];
        let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
        let path = shape_path(ShapeKind::CustomPath, &commands, bounds);
        let inside = objc2_core_foundation::CGPoint::new(75.0, 25.0);
        // SAFETY: `path` is a live path built above; null transform.
        assert!(unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), inside, false) });
        let outside = objc2_core_foundation::CGPoint::new(25.0, 75.0);
        // SAFETY: same.
        assert!(!unsafe { CGPath::contains_point(Some(&path), core::ptr::null(), outside, false) });
    }
}
