//! The `scroll` leaf: `Native<ScrollView>` rendered through the kit's
//! scroll surface.
//!
//! Mirrors `WuiScroll`: the scroll view claims the space it is offered and
//! answers `0` on a min query it cannot satisfy, `stretchAxis` is `.both`,
//! the content keeps its measured extent on the non-scrolling axis — centred
//! in the viewport and clipped on both edges when wider — and the scrollable
//! extent is the placement's answer. The controller drives the offset
//! through generation bumps; the offset binding reports every scroll,
//! user- or controller-driven, as the inset-adjusted scroll position.
//!
//! Contract friction: `SubView` has no `place` hook, so the constructed
//! cross-axis proposal the Swift port delivered through
//! `setPlacementProposal` has no channel here — children measure under the
//! proposals the layout handler issues instead, like every other ported
//! container.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::{MainThreadMarker, Retained};
use waterui::layout::scroll::{Axis, ScrollController, ScrollView};
use waterui::reactive::{Binding, Signal};
use waterui_core::layout::{Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
mod platform {
    pub(super) use cocoa_ui::appkit::ScrollView;
}

#[cfg(target_os = "ios")]
mod platform {
    pub(super) use cocoa_ui::uikit::ScrollView;
}

use cocoa_ui::PlatformView;
use platform::ScrollView as ScrollSurface;

/// The scroll view as its platform view.
fn as_view(scroll: &ScrollSurface) -> &PlatformView {
    scroll
}

/// The scroll's answer to a size proposal, matching `scrollMinSize`.
///
/// A finite proposal is claimed. An unspecified axis asks for the content's
/// intrinsic extent — the measure runs only when an axis is actually
/// unspecified, so a fully finite proposal never reads the answer. A min
/// (zero) query answers the content's minimum on the non-scrolling axis and
/// `0` on the scrolling one.
fn scroll_min_size(
    axis: Axis,
    proposal: ProposalSize,
    measure_content: impl Fn(ProposalSize) -> Size,
) -> Size {
    let min_width = proposal.width == Some(0.0);
    let min_height = proposal.height == Some(0.0);
    let dimension = |value: Option<f32>| value.unwrap_or(0.0);
    if !min_width && !min_height {
        if proposal.width.is_some() && proposal.height.is_some() {
            return Size::new(dimension(proposal.width), dimension(proposal.height));
        }
        let intrinsic = measure_content(ProposalSize::UNSPECIFIED);
        return Size::new(
            proposal.width.unwrap_or(intrinsic.width),
            proposal.height.unwrap_or(intrinsic.height),
        );
    }
    let content_minimum = |content_proposal: ProposalSize| {
        let minimum = measure_content(content_proposal);
        Size::new(
            if minimum.width.is_finite() {
                minimum.width.max(0.0)
            } else {
                0.0
            },
            if minimum.height.is_finite() {
                minimum.height.max(0.0)
            } else {
                0.0
            },
        )
    };
    match axis {
        Axis::Vertical => Size::new(
            if min_width {
                content_minimum(ProposalSize::new(Some(0.0), None)).width
            } else {
                dimension(proposal.width)
            },
            if min_height {
                0.0
            } else {
                dimension(proposal.height)
            },
        ),
        Axis::Horizontal => Size::new(
            if min_width {
                0.0
            } else {
                dimension(proposal.width)
            },
            if min_height {
                content_minimum(ProposalSize::new(None, Some(0.0))).height
            } else {
                dimension(proposal.height)
            },
        ),
        Axis::All => Size::new(
            if min_width {
                0.0
            } else {
                dimension(proposal.width)
            },
            if min_height {
                0.0
            } else {
                dimension(proposal.height)
            },
        ),
        _ => panic!("unsupported WaterUI scroll axis: {axis:?}"),
    }
}

/// Measures scroll content, then measures it again under its own answer
/// when the answer exceeds the viewport on the non-scrolling axis, matching
/// `scrollMeasuredContent`: the second pass measures under the frame the
/// content is placed in, so the scrollable extent matches the layout that
/// renders.
fn scroll_measured_content(
    axis: Axis,
    viewport: Size,
    measure: impl Fn(ProposalSize) -> Size,
) -> Size {
    match axis {
        Axis::Vertical => {
            let mut measured = measure(ProposalSize::new(Some(viewport.width), None));
            if measured.width > viewport.width {
                measured = measure(ProposalSize::new(Some(measured.width), None));
            }
            measured
        }
        Axis::Horizontal => {
            let mut measured = measure(ProposalSize::new(None, Some(viewport.height)));
            if measured.height > viewport.height {
                measured = measure(ProposalSize::new(None, Some(measured.height)));
            }
            measured
        }
        Axis::All => {
            let mut measured = measure(ProposalSize::UNSPECIFIED);
            if measured.width > viewport.width {
                measured = measure(ProposalSize::new(Some(measured.width), None));
            }
            if measured.height > viewport.height {
                measured = measure(ProposalSize::new(None, Some(measured.height)));
            }
            measured
        }
        _ => panic!("unsupported WaterUI scroll axis: {axis:?}"),
    }
}

/// Where a scroll frames its content once the content has answered the
/// viewport-constrained proposal — `scrollContentPlacement`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScrollContentPlacement {
    /// The content view's frame in the scroll view's coordinate space.
    content_frame: Rect,
    /// The scrollable extent: `contentSize` on `UIKit`, the document frame
    /// on `AppKit`.
    scroll_extent: Size,
}

/// On the non-scrolling axis the content keeps its own answer and is
/// centred in the viewport; the scrollable extent stays the viewport's so
/// nothing scrolls sideways.
fn scroll_content_placement(axis: Axis, viewport: Size, measured: Size) -> ScrollContentPlacement {
    match axis {
        Axis::Vertical => ScrollContentPlacement {
            content_frame: Rect::new(
                Point::new((viewport.width - measured.width) / 2.0, 0.0),
                measured,
            ),
            scroll_extent: Size::new(viewport.width, measured.height),
        },
        Axis::Horizontal => ScrollContentPlacement {
            content_frame: Rect::new(
                Point::new(0.0, (viewport.height - measured.height) / 2.0),
                measured,
            ),
            scroll_extent: Size::new(measured.width, viewport.height),
        },
        Axis::All => ScrollContentPlacement {
            content_frame: Rect::new(Point::zero(), measured),
            scroll_extent: measured,
        },
        _ => panic!("unsupported WaterUI scroll axis: {axis:?}"),
    }
}

/// The shared state a scroll leaf hands to its layout face and to the kit's
/// layout handler: the mounted child, the axis, and the last-laid-out
/// viewport (`AppKit`'s `tile()` retry).
struct ScrollContent {
    axis: Axis,
    child: Mounted,
    laid_out_viewport: Cell<Size>,
}

/// The leaf's layout face: the scroll's own proposal answers.
struct ScrollSubView {
    content: Rc<ScrollContent>,
}

impl SubView for ScrollSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let child = &self.content.child;
        ViewDimensions::new(scroll_min_size(
            self.content.axis,
            proposal,
            |content_proposal| child.layout().measure(content_proposal).size,
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The scroll surface's viewport as a layout size.
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; layout proposals are f32"
)]
fn layout_viewport(scroll: &ScrollSurface) -> Size {
    let viewport = scroll.viewport_size();
    Size::new(viewport.width as f32, viewport.height as f32)
}

/// Measures the mounted child and frames it per the placement.
fn place_child(content: &ScrollContent, scroll: &ScrollSurface) {
    let viewport = layout_viewport(scroll);
    let measured = scroll_measured_content(content.axis, viewport, |proposal| {
        content.child.layout().measure(proposal).size
    });
    let placement = scroll_content_placement(content.axis, viewport, measured);
    layout_child(content, scroll, placement);
}

/// `UIKit`: the child mounts on the scroll view itself; the scrollable
/// extent is `contentSize` and the frame only changes hands when it
/// changed, avoiding recursive layout.
#[cfg(target_os = "ios")]
fn layout_child(
    content: &ScrollContent,
    scroll: &ScrollSurface,
    placement: ScrollContentPlacement,
) {
    let child_view = content.child.view();
    let origin = placement.content_frame.origin();
    let size = placement.content_frame.size();
    let frame = cocoa_ui::Rect::new(
        f64::from(origin.x),
        f64::from(origin.y),
        f64::from(size.width),
        f64::from(size.height),
    );
    if cocoa_ui::Rect::from(child_view.frame()) != frame {
        cocoa_ui::view::set_frame(child_view, frame);
        scroll.set_content_extent(cocoa_ui::Size::new(
            f64::from(placement.scroll_extent.width),
            f64::from(placement.scroll_extent.height),
        ));
        child_view.setNeedsLayout();
        child_view.layoutIfNeeded();
    }
}

/// `AppKit`: the child mounts on the flipped document view, which the
/// placement frames to the scroll extent every pass.
#[cfg(target_os = "macos")]
fn layout_child(
    content: &ScrollContent,
    scroll: &ScrollSurface,
    placement: ScrollContentPlacement,
) {
    scroll.set_document_extent(cocoa_ui::Size::new(
        f64::from(placement.scroll_extent.width),
        f64::from(placement.scroll_extent.height),
    ));
    let origin = placement.content_frame.origin();
    let size = placement.content_frame.size();
    let child_view = content.child.view();
    cocoa_ui::view::set_frame(
        child_view,
        cocoa_ui::Rect::new(
            f64::from(origin.x),
            f64::from(origin.y),
            f64::from(size.width),
            f64::from(size.height),
        ),
    );
    child_view.setNeedsLayout(true);
    child_view.layoutSubtreeIfNeeded();
}

/// `UIKit`: applies a controller target, clamped inside the adjusted
/// content insets, matching `applyScrollControllerTarget`.
#[cfg(target_os = "ios")]
fn apply_scroll_target(scroll: &ScrollSurface, target: Point) {
    scroll.layout_if_needed();
    assert!(
        target.x.is_finite() && target.y.is_finite(),
        "WaterUI ScrollView target must contain finite coordinates"
    );
    let inset = scroll.adjusted_content_inset();
    let extent = scroll.content_extent();
    let viewport = scroll.viewport_size();
    let minimum_x = -inset.left;
    let minimum_y = -inset.top;
    let maximum_x = (extent.width - viewport.width + inset.right).max(minimum_x);
    let maximum_y = (extent.height - viewport.height + inset.bottom).max(minimum_y);
    scroll.set_content_offset(
        cocoa_ui::Point::new(
            (f64::from(target.x) - inset.left).clamp(minimum_x, maximum_x),
            (f64::from(target.y) - inset.top).clamp(minimum_y, maximum_y),
        ),
        false,
    );
}

/// `AppKit`: applies a controller target by scrolling the clip view, like
/// `applyScrollControllerTarget`.
#[cfg(target_os = "macos")]
fn apply_scroll_target(scroll: &ScrollSurface, target: Point) {
    scroll.layout_if_needed();
    assert!(
        target.x.is_finite() && target.y.is_finite(),
        "WaterUI ScrollView target must contain finite coordinates"
    );
    scroll.scroll_to(cocoa_ui::Point::new(
        f64::from(target.x.max(0.0)),
        f64::from(target.y.max(0.0)),
    ));
}

/// Renders a `ScrollView` config into the kit's scroll surface.
fn render(config: ScrollView, ctx: &RenderContext<'_>) -> NativeLeaf {
    let parts = config.into_inner();
    let mtm = ctx.mtm();
    let (vertical, horizontal) = match parts.axis {
        Axis::Vertical => (true, false),
        Axis::Horizontal => (false, true),
        Axis::All => (true, true),
        axis => panic!("unsupported WaterUI scroll axis: {axis:?}"),
    };
    let scroll = ScrollSurface::new(mtm, vertical, horizontal);
    let child_leaf = ctx.render(parts.content);
    // `AppKit` mounts on the flipped document; `UIKit` scrolls the view's
    // own content area, so the child mounts on the scroll view itself.
    #[cfg(target_os = "macos")]
    let child = child_leaf.mount(
        &scroll
            .document_view()
            .expect("the kit scroll view always installs its document"),
    );
    #[cfg(target_os = "ios")]
    let child = child_leaf.mount(as_view(&scroll));
    let content = Rc::new(ScrollContent {
        axis: parts.axis,
        child,
        laid_out_viewport: Cell::new(Size::zero()),
    });

    scroll.set_layout_handler({
        let content = Rc::clone(&content);
        move |scroll| {
            content.laid_out_viewport.set(layout_viewport(scroll));
            place_child(&content, scroll);
        }
    });

    // `AppKit` retiles when a scroller appears or disappears; a clip area
    // the content has not been laid out against asks for another pass.
    #[cfg(target_os = "macos")]
    scroll.set_tile_handler({
        let content = Rc::clone(&content);
        move |scroll| {
            if layout_viewport(scroll) != content.laid_out_viewport.get() {
                scroll.set_needs_layout();
                crate::measure_memo::invalidate();
            }
        }
    });

    let mut leaf = NativeLeaf::new(
        as_view(&scroll),
        ScrollSubView {
            content: Rc::clone(&content),
        },
    );
    leaf.keep(content);

    if let Some(controller) = parts.controller {
        wire_controller(&mut leaf, &scroll, &controller);
    }

    if let Some(offset) = parts.offset {
        wire_offset_reporting(&mut leaf, &scroll, offset, mtm);
    }

    leaf
}

/// Wires a `ScrollController` into the scroll surface: a generation bump
/// applies the current target. The Swift port kept the target observations
/// alive with no-op watchers and read the latest value at apply time — the
/// snapshot here is that same read.
fn wire_controller(
    leaf: &mut NativeLeaf,
    scroll: &cocoa_ui::Retained<ScrollSurface>,
    controller: &ScrollController<Point>,
) {
    let target = controller.target();
    let generation = controller.generation();
    leaf.watch(&target, |_| {});
    if generation.snapshot() > 0 {
        apply_scroll_target(scroll, target.snapshot());
    }
    leaf.watch(&generation, {
        let scroll = Retained::clone(scroll);
        let target = target.clone();
        move |ctx| {
            if *ctx.value() > 0 {
                apply_scroll_target(&scroll, target.snapshot());
            }
        }
    });
}

/// `UIKit`: every scroll reports the inset-adjusted content offset, so a
/// rest position reports zero and a controller target reports its own
/// coordinate.
#[cfg(target_os = "ios")]
fn wire_offset_reporting(
    _leaf: &mut NativeLeaf,
    scroll: &Retained<ScrollSurface>,
    offset: Binding<Point>,
    _mtm: MainThreadMarker,
) {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "kit geometry is f64; the offset binding is f32"
    )]
    let report = move |scroll: &ScrollSurface| {
        let position = scroll.content_offset();
        let inset = scroll.adjusted_content_inset();
        offset.set(Point::new(
            (position.x + inset.left) as f32,
            (position.y + inset.top) as f32,
        ));
    };
    scroll.on_scroll({
        let report = report.clone();
        move |scroll| report(scroll)
    });
    report(scroll);
}

/// `AppKit`: the clip view's bounds origin is the content offset; it
/// changes on every scroll, whether from the user or a controller request.
#[cfg(target_os = "macos")]
fn wire_offset_reporting(
    leaf: &mut NativeLeaf,
    scroll: &Retained<ScrollSurface>,
    offset: Binding<Point>,
    mtm: MainThreadMarker,
) {
    scroll.clip_view().setPostsBoundsChangedNotifications(true);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "kit geometry is f64; the offset binding is f32"
    )]
    let report = {
        let scroll = Retained::clone(scroll);
        move || {
            let origin = scroll.content_offset();
            offset.set(Point::new(origin.x as f32, origin.y as f32));
        }
    };
    leaf.keep(cocoa_ui::notification::observe_object(
        mtm,
        // SAFETY: reads a notification-name constant `AppKit` owns for the
        // process's lifetime.
        &cocoa_ui::notification::NotificationName::framework(unsafe {
            cocoa_ui::objc2_app_kit::NSViewBoundsDidChangeNotification
        }),
        scroll.clip_view().as_ref(),
        {
            let report = report.clone();
            move || report()
        },
    ));
    report();
}

/// Registers the `scroll` leaf.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<ScrollView>(render);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewport() -> Size {
        Size::new(402.0, 600.0)
    }

    #[test]
    fn vertical_scroll_centres_wider_content_across_both_edges() {
        let placement =
            scroll_content_placement(Axis::Vertical, viewport(), Size::new(414.0, 384.0));
        assert_eq!(
            placement.content_frame,
            Rect::new(Point::new(-6.0, 0.0), Size::new(414.0, 384.0))
        );
        assert_eq!(placement.scroll_extent, Size::new(402.0, 384.0));
    }

    #[test]
    fn vertical_scroll_centres_narrower_content() {
        let placement =
            scroll_content_placement(Axis::Vertical, viewport(), Size::new(200.0, 96.0));
        assert_eq!(
            placement.content_frame,
            Rect::new(Point::new(101.0, 0.0), Size::new(200.0, 96.0))
        );
        assert_eq!(placement.scroll_extent, Size::new(402.0, 96.0));
    }

    #[test]
    fn horizontal_scroll_centres_on_the_vertical_axis() {
        let placement =
            scroll_content_placement(Axis::Horizontal, viewport(), Size::new(1200.0, 640.0));
        assert_eq!(
            placement.content_frame,
            Rect::new(Point::new(0.0, -20.0), Size::new(1200.0, 640.0))
        );
        assert_eq!(placement.scroll_extent, Size::new(1200.0, 600.0));
    }

    #[test]
    fn bidirectional_scroll_frames_the_answer_at_the_origin() {
        let placement = scroll_content_placement(Axis::All, viewport(), Size::new(1200.0, 900.0));
        assert_eq!(
            placement.content_frame,
            Rect::new(Point::zero(), Size::new(1200.0, 900.0))
        );
        assert_eq!(placement.scroll_extent, Size::new(1200.0, 900.0));
    }

    /// A paragraph: unwrapped when unspecified, wrapped to the offer, and
    /// at its longest word for a zero offer.
    fn paragraph(proposal: ProposalSize) -> Size {
        match proposal.width {
            None => Size::new(2200.0, 20.0),
            Some(0.0) => Size::new(48.0, 900.0),
            Some(width) => Size::new(width, 20.0 * (2200.0 / width).ceil()),
        }
    }

    #[test]
    fn vertical_scroll_min_width_is_the_contents_narrowest_wrap() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(0.0), Some(600.0)),
            paragraph,
        );
        assert_eq!(size, Size::new(48.0, 600.0));
    }

    #[test]
    fn vertical_scroll_min_height_is_zero() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(402.0), Some(0.0)),
            paragraph,
        );
        assert_eq!(size, Size::new(402.0, 0.0));
    }

    #[test]
    fn horizontal_scroll_min_height_is_the_contents_minimum() {
        let size = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(Some(402.0), Some(0.0)),
            |proposal| {
                if proposal.height == Some(0.0) {
                    Size::new(900.0, 32.0)
                } else {
                    Size::new(900.0, 200.0)
                }
            },
        );
        assert_eq!(size, Size::new(402.0, 32.0));
    }

    /// An unspecified axis asks for the ideal extent, which the contract
    /// orders at or above the minimum — the scroll answers the content's
    /// intrinsic extent there and still claims the finite offer on the
    /// other axis.
    #[test]
    fn unspecified_axis_answers_the_contents_intrinsic_extent() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(None, Some(80.0)),
            paragraph,
        );
        assert_eq!(size, Size::new(2200.0, 80.0));

        let twin = scroll_min_size(Axis::Vertical, ProposalSize::new(None, Some(80.0)), |_| {
            Size::new(60.0, 300.0)
        });
        assert_eq!(twin, Size::new(60.0, 80.0));
    }
}
