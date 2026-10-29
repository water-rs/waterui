//! The `scroll` leaf: `Native<ScrollView>` rendered through the kit's
//! scroll surface.
//!
//! Mirrors `WuiScroll`: the scroll view claims the space it is offered on
//! its scrolling axis, answers `0` on a min query it cannot satisfy, and
//! stretches on the non-scrolling axis only when its content does; the
//! content keeps its measured extent on the non-scrolling axis — centred
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
use waterui::animation::Animation;
use waterui::layout::scroll::{
    Axis, ScrollController, ScrollPosition, ScrollRequest, ScrollView, scroll_stretch_axis,
};
use waterui::reactive::{Binding, Signal};
use waterui_core::layout::{Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::{Mounted, NativeLeaf, RenderContext};
use crate::dispatch::Dispatcher;
use crate::proposal;

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

/// The scroll's answer to a size proposal, matching `scrollMinSize`
/// (layout-spec.md §6, water-rs/waterui#1208).
///
/// On the scrolling axis a finite proposal is claimed, `None` asks for the
/// content's intrinsic extent and `0` answers `0`. On the non-scrolling axis
/// the scroll answers the content's own answer to the content proposal —
/// the offer on that axis, `None` on the scrolling axis — clamped to a
/// finite offer, so a scroll hugs non-filling content; `0` and `None` keep
/// their existing behaviour (the content's minimum and intrinsic extent).
/// The one content measure under that content proposal serves the
/// non-scrolling answer and an unspecified scrolling-axis extent together.
///
/// `reservation` is what the platform's scrollers take out of the frame
/// beside the viewport ([`scroller_reservation`]). The content is offered
/// the viewport — the offer less the reservation — and every answer derived
/// from the content adds the reservation back, so the viewport, not the
/// frame, holds the content's answer: a hugging scroll beside a legacy
/// `AppKit` scroller is the scroller's width wider than its content, the
/// way `NSScrollView`'s own `frameSizeForContentSize:` and `SwiftUI`'s
/// `ScrollView` size it (water-rs/waterui#3597).
fn scroll_min_size(
    axis: Axis,
    proposal: ProposalSize,
    reservation: Size,
    measure_content: impl Fn(ProposalSize) -> Size,
) -> Size {
    // A `0` proposal measures the content's minimum: clamp a non-finite or
    // negative answer out of the layout arithmetic.
    let minimum = |value: f32| {
        if value.is_finite() {
            value.max(0.0)
        } else {
            0.0
        }
    };
    // A finite offer caps the content's own answer on the non-scrolling
    // axis; an unbounded answer fills the offer.
    let cross = |answer: f32, offer: f32| {
        if answer.is_finite() {
            answer.clamp(0.0, offer)
        } else {
            offer
        }
    };
    // The viewport an offer leaves once the scrollers took their share.
    let viewport =
        |offer: Option<f32>, reserved: f32| offer.map(|offer| (offer - reserved).max(0.0));
    match axis {
        Axis::Vertical => {
            // The §6 content proposal for a vertical scroll: the width offer
            // on the content, its height free.
            let measured = measure_content(ProposalSize::new(
                viewport(proposal.width, reservation.width),
                None,
            ));
            Size::new(
                match proposal.width {
                    Some(0.0) => minimum(measured.width) + reservation.width,
                    Some(offer) => cross(measured.width + reservation.width, offer),
                    None => measured.width + reservation.width,
                },
                match proposal.height {
                    Some(0.0) => 0.0,
                    Some(height) => height,
                    None => measured.height + reservation.height,
                },
            )
        }
        Axis::Horizontal => {
            let measured = measure_content(ProposalSize::new(
                None,
                viewport(proposal.height, reservation.height),
            ));
            Size::new(
                match proposal.width {
                    Some(0.0) => 0.0,
                    Some(width) => width,
                    None => measured.width + reservation.width,
                },
                match proposal.height {
                    Some(0.0) => minimum(measured.height) + reservation.height,
                    Some(offer) => cross(measured.height + reservation.height, offer),
                    None => measured.height + reservation.height,
                },
            )
        }
        Axis::All => {
            // `Axis::All` is unchanged: both axes scroll, so a finite
            // proposal is claimed, `None` asks for the intrinsic extent and
            // `0` answers `0`.
            let measured = (proposal.width.is_none() || proposal.height.is_none())
                .then(|| measure_content(ProposalSize::UNSPECIFIED));
            Size::new(
                match proposal.width {
                    Some(0.0) => 0.0,
                    Some(width) => width,
                    None => measured.map_or(0.0, |measured| measured.width + reservation.width),
                },
                match proposal.height {
                    Some(0.0) => 0.0,
                    Some(height) => height,
                    None => measured.map_or(0.0, |measured| measured.height + reservation.height),
                },
            )
        }
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
/// layout handler: the mounted child, the axis, the last-laid-out
/// viewport (`AppKit`'s `tile()` retry), and the offset the parent's pixel
/// alignment of the scroll view left, which the content aligns under.
struct ScrollContent {
    axis: Axis,
    child: Mounted,
    laid_out_viewport: Cell<Size>,
    offset: Cell<cocoa_ui::view::SubpixelOffset>,
}

/// The leaf's layout face: the scroll's own proposal answers.
struct ScrollSubView {
    content: Rc<ScrollContent>,
    /// Read for the scrollers' reservation at every measure, since the
    /// scroller style can change while the scroll is mounted.
    surface: Retained<ScrollSurface>,
}

impl SubView for ScrollSubView {
    fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
        let child = &self.content.child;
        ViewDimensions::new(scroll_min_size(
            self.content.axis,
            proposal,
            scroller_reservation(&self.surface),
            |content_proposal| child.layout().measure(content_proposal).size,
        ))
    }

    fn stretch_axis(&self) -> StretchAxis {
        scroll_stretch_axis(
            self.content.axis,
            self.content.child.layout().stretch_axis(),
        )
    }

    fn priority(&self) -> i32 {
        0
    }
}

/// The extent `AppKit`'s scrollers take out of the scroll's frame beside
/// the viewport — the legacy scroller's track, nothing for overlay
/// scrollers — as a layout size.
#[cfg(target_os = "macos")]
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; layout proposals are f32"
)]
fn scroller_reservation(scroll: &ScrollSurface) -> Size {
    let reserved = scroll.scroller_reservation();
    Size::new(reserved.width as f32, reserved.height as f32)
}

/// `UIKit`'s scroll indicators float over the content and reserve none of
/// the frame.
#[cfg(target_os = "ios")]
const fn scroller_reservation(_scroll: &ScrollSurface) -> Size {
    Size::zero()
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
    let previous = cocoa_ui::Rect::from(child_view.frame());
    // The scroll measures its content itself and negotiates no proposal.
    let written = proposal::place(child_view, frame, None, content.offset.get());
    if previous != written {
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
    // The scroll measures its content itself and negotiates no proposal.
    proposal::place(
        child_view,
        cocoa_ui::Rect::new(
            f64::from(origin.x),
            f64::from(origin.y),
            f64::from(size.width),
            f64::from(size.height),
        ),
        None,
        content.offset.get(),
    );
    child_view.setNeedsLayout(true);
    child_view.layoutSubtreeIfNeeded();
}

/// `UIKit`: the content offset `target` resolves to — clamped inside the
/// adjusted content insets, the same math the jump applies, shared by
/// every animated variant.
#[cfg(target_os = "ios")]
fn clamped_offset(scroll: &ScrollSurface, target: Point) -> cocoa_ui::Point {
    scroll.clamped_content_offset(raw_content_offset(target, scroll.adjusted_content_inset()))
}

/// `UIKit`: applies a controller request — `None` is the jump it always
/// was, `Animation::Default` is `UIKit`'s own animated `contentOffset`
/// write, and an explicit animation drives the offset on the frame clock,
/// writing the model along the core curve each tick. The clamped target
/// is the jump's, so the flight lands where it would have.
#[cfg(target_os = "ios")]
fn apply_scroll_request(scroll: &ScrollSurface, request: &ScrollRequest<Point>) {
    scroll.layout_if_needed();
    assert!(
        request.target.x.is_finite() && request.target.y.is_finite(),
        "WaterUI ScrollView target must contain finite coordinates"
    );
    let offset = clamped_offset(scroll, request.target);
    match request.animation.as_ref() {
        None => scroll.set_content_offset(offset, false),
        Some(Animation::Default) => scroll.set_content_offset(offset, true),
        Some(animation) => scroll.animate_content_offset(
            offset,
            animation.duration().as_secs_f64(),
            crate::animation::progress(animation),
        ),
    }
}

/// `AppKit`: applies a controller request, its target relative to the inset
/// content edge like `UIKit`'s — `None` jumps the clip view as
/// `applyScrollControllerTarget` did, `Animation::Default` plays the
/// system's animated scroll (`boundsOrigin` through the clip's animator
/// proxy in an `NSAnimationContext` group, the `AppKit` counterpart of
/// `setContentOffset(_:animated: true)`), and an explicit animation drives
/// the offset on the frame clock, writing the model along the core curve
/// each tick.
#[cfg(target_os = "macos")]
fn apply_scroll_request(scroll: &ScrollSurface, request: &ScrollRequest<Point>) {
    scroll.layout_if_needed();
    assert!(
        request.target.x.is_finite() && request.target.y.is_finite(),
        "WaterUI ScrollView target must contain finite coordinates"
    );
    // The target is relative to the inset content edge; the kit's jump and
    // flights clamp the raw offset to the clip's scrollable bounds.
    let target = raw_content_offset(request.target, scroll.adjusted_content_inset());
    match request.animation.as_ref() {
        None => scroll.scroll_to(target),
        Some(Animation::Default) => scroll.scroll_to_animated(target),
        Some(animation) => scroll.animate_scroll_to(
            target,
            animation.duration().as_secs_f64(),
            crate::animation::progress(animation),
        ),
    }
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
        offset: Cell::new(cocoa_ui::view::SubpixelOffset::ZERO),
    });

    // `measured` flips when the first layout completes `place_child` —
    // until then the surface's geometry is placeholder, and the position
    // sink must not see a fabricated `Some` ahead of real layout. The
    // offset binding keeps its existing report-immediately semantics.
    let measured = Rc::new(Cell::new(false));
    // One reporter serves both sinks: `report_offset`'s binding and the
    // controller's position report. Every write is deduped against the
    // binding's current value so the layout handler and the scroll observer
    // may both fire it without double-notifying.
    let report = {
        let offset = parts.offset.clone();
        let position = parts
            .controller
            .as_ref()
            .map(ScrollController::position_report);
        (offset.is_some() || position.is_some())
            .then(|| scroll_reporter(parts.axis, offset, position, Rc::clone(&measured)))
    };
    scroll.set_layout_handler({
        // Strong capture, the way mounted `HostView` handlers capture
        // their state since water-rs/waterui#1860: the scroll view's
        // handler slot holds `ScrollContent`, which holds the mounted
        // child leaf — so while the leaf lives, the scroll view retains
        // the whole content subtree through the handler. The
        // `HandlerTeardown` guard kept below clears the slot at the
        // leaf's release boundary; a layout pass the platform delivers
        // to a view that outlives the leaf finds `None`.
        let content = Rc::clone(&content);
        let report = report.clone();
        let measured = Rc::clone(&measured);
        move |scroll| {
            content.laid_out_viewport.set(layout_viewport(scroll));
            place_child(&content, scroll);
            // The extent `place_child` just committed — and any viewport
            // resize that triggered the layout — is real geometry: mark
            // measured, then report, so a resize or content growth reaches
            // the controller without a scroll happening.
            measured.set(true);
            if let Some(report) = &report {
                report(scroll);
            }
        }
    });

    // `AppKit` retiles when a scroller appears or disappears; a clip area
    // the content has not been laid out against asks for another pass. A
    // scroller style change (the system's "Show scroll bars" setting) also
    // changes the reservation the scroll's own answer carries, so the
    // ancestors negotiate the scroll's frame again.
    #[cfg(target_os = "macos")]
    scroll.set_tile_handler({
        // The same strong capture and the same release boundary as the
        // layout handler above.
        let content = Rc::clone(&content);
        let reservation = Cell::new(scroller_reservation(&scroll));
        move |scroll| {
            let reserved = scroller_reservation(scroll);
            if reserved != reservation.replace(reserved) {
                crate::measure_memo::invalidate();
                cocoa_ui::view::invalidate_layout(as_view(scroll));
            } else if layout_viewport(scroll) != content.laid_out_viewport.get() {
                scroll.set_needs_layout();
                crate::measure_memo::invalidate();
            }
        }
    });

    // A parent's placement moves the content's pixel alignment even when
    // the scroll's frame does not change.
    let sink_guard = proposal::register_sink(as_view(&scroll), {
        let content = Rc::clone(&content);
        let scroll = Retained::clone(&scroll);
        move |placement| {
            if content.offset.get() != placement.offset {
                content.offset.set(placement.offset);
                scroll.set_needs_layout();
            }
        }
    });

    let mut leaf = NativeLeaf::new(
        as_view(&scroll),
        ScrollSubView {
            content: Rc::clone(&content),
            surface: Retained::clone(&scroll),
        },
    );
    leaf.keep(sink_guard);
    // The scroll view's handler slots hold `content` (and the mounted
    // child inside it) while the leaf lives; the guard clears those
    // slots when the leaf's keepalive drops — the leaf's release
    // boundary — so a scroll view retained past the leaf cannot keep the
    // content subtree alive.
    leaf.keep(cocoa_ui::HandlerTeardown::new(Retained::clone(&scroll)));

    if let Some(controller) = parts.controller {
        wire_controller(&mut leaf, &scroll, &controller);
    }

    if let Some(report) = &report {
        wire_scroll_reporting(&mut leaf, &scroll, report, mtm);
    }

    leaf
}

/// Wires a `ScrollController` into the scroll surface: a generation bump
/// applies the latest request — target and animation together. The Swift
/// port kept the target observations alive with no-op watchers and read
/// the latest value at apply time — the snapshot here is that same read.
fn wire_controller(
    leaf: &mut NativeLeaf,
    scroll: &cocoa_ui::Retained<ScrollSurface>,
    controller: &ScrollController<Point>,
) {
    let request = controller.request();
    let generation = controller.generation();
    leaf.watch(&request, |_| {});
    if generation.snapshot() > 0 {
        apply_scroll_request(scroll, &request.snapshot());
    }
    leaf.watch(&generation, {
        let scroll = Retained::clone(scroll);
        let request = request.clone();
        move |ctx| {
            if *ctx.value() > 0 {
                apply_scroll_request(&scroll, &request.snapshot());
            }
        }
    });
}

/// The scroll offset relative to the inset content edge: the platform's raw
/// content offset plus the insets on the leading edges, so a surface at its
/// top reports zero on both platforms.
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; the report bindings are f32"
)]
fn inset_relative_offset(raw: cocoa_ui::Point, inset: cocoa_ui::EdgeInsets) -> Point {
    Point::new((raw.x + inset.left) as f32, (raw.y + inset.top) as f32)
}

/// The inverse of [`inset_relative_offset`]: the platform's raw content
/// offset an inset-relative `target` resolves to, before the platform's own
/// clamp.
fn raw_content_offset(target: Point, inset: cocoa_ui::EdgeInsets) -> cocoa_ui::Point {
    cocoa_ui::Point::new(
        f64::from(target.x) - inset.left,
        f64::from(target.y) - inset.top,
    )
}

/// The viewport excluding the insets.
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; the report bindings are f32"
)]
fn inset_viewport(viewport: cocoa_ui::Size, inset: cocoa_ui::EdgeInsets) -> Size {
    Size::new(
        (viewport.width - inset.left - inset.right).max(0.0) as f32,
        (viewport.height - inset.top - inset.bottom).max(0.0) as f32,
    )
}

/// Reads the surface's completed geometry into the report sinks — the
/// offset relative to the inset content edge, the viewport excluding the
/// insets, and the content extent as the platform keeps it, so `at_end`
/// lands where the platform's own maximum does. Both writes dedupe against
/// the binding's current value; nothing here reads a pending request.
#[expect(
    clippy::cast_possible_truncation,
    reason = "kit geometry is f64; the report bindings are f32"
)]
fn scroll_reporter(
    axis: Axis,
    offset: Option<Binding<Point>>,
    position: Option<Binding<Option<ScrollPosition>>>,
    measured: Rc<Cell<bool>>,
) -> Rc<dyn Fn(&ScrollSurface)> {
    Rc::new(move |scroll| {
        let adjusted =
            inset_relative_offset(scroll.content_offset(), scroll.adjusted_content_inset());
        if let Some(offset) = &offset
            && offset.snapshot() != adjusted
        {
            offset.set(adjusted);
        }
        if let Some(sink) = &position
            && measured.get()
        {
            // The offset write just ran watchers that may have scrolled:
            // re-read the surface so the position snapshot is coherent.
            let inset = scroll.adjusted_content_inset();
            let extent = scroll.content_extent();
            let current = ScrollPosition::new(
                axis,
                inset_relative_offset(scroll.content_offset(), inset),
                inset_viewport(scroll.viewport_size(), inset),
                Size::new(extent.width as f32, extent.height as f32),
            );
            if sink.snapshot() != Some(current) {
                sink.set(Some(current));
            }
        }
    })
}

/// `UIKit`: the scroll observer fires the reporter on every scroll —
/// user-, flight- or controller-driven — and once now for the surface's
/// initial geometry. Layout-driven changes reach it through the layout
/// handler registered in `render`.
#[cfg(target_os = "ios")]
fn wire_scroll_reporting(
    _leaf: &mut NativeLeaf,
    scroll: &Retained<ScrollSurface>,
    report: &Rc<dyn Fn(&ScrollSurface)>,
    _mtm: MainThreadMarker,
) {
    scroll.on_scroll({
        let report = report.clone();
        move |scroll| report(scroll)
    });
    report(scroll);
}

/// `AppKit`: the clip view's bounds change on every scroll and resize —
/// the observer fires the reporter for both, and once now for the
/// surface's initial geometry. Content-extent changes reach it through
/// the layout handler registered in `render`.
#[cfg(target_os = "macos")]
fn wire_scroll_reporting(
    leaf: &mut NativeLeaf,
    scroll: &Retained<ScrollSurface>,
    report: &Rc<dyn Fn(&ScrollSurface)>,
    mtm: MainThreadMarker,
) {
    scroll.clip_view().setPostsBoundsChangedNotifications(true);
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
            let scroll = Retained::clone(scroll);
            move || report(&scroll)
        },
    ));
    report(scroll);
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

    /// Scrollers that float over the content: overlay `AppKit` scrollers
    /// and `UIKit` indicators.
    const NO_SCROLLERS: Size = Size::zero();

    /// A legacy `AppKit` vertical scroller: a regular scroller's 17 pt
    /// track beside the viewport on macOS 26.
    const LEGACY_VERTICAL_SCROLLER: Size = Size::new(17.0, 0.0);

    /// A toolbar band over the top of a surface, as the kit's content insets
    /// carry it.
    fn toolbar_inset() -> cocoa_ui::EdgeInsets {
        cocoa_ui::EdgeInsets::new(52.0, 0.0, 0.0, 0.0)
    }

    /// A surface at rest at its top rests at the negated inset on `AppKit`
    /// (and `UIKit`); it reports zero (water-rs/waterui#3674).
    #[test]
    fn surface_at_its_top_reports_offset_zero() {
        let inset = toolbar_inset();
        let raw = cocoa_ui::Point::new(0.0, -inset.top);
        assert_eq!(inset_relative_offset(raw, inset), Point::zero());
    }

    #[test]
    fn viewport_excludes_the_insets() {
        let inset = cocoa_ui::EdgeInsets::new(52.0, 4.0, 10.0, 6.0);
        assert_eq!(
            inset_viewport(cocoa_ui::Size::new(402.0, 600.0), inset),
            Size::new(392.0, 538.0)
        );
    }

    /// A controller target is read in the same space the offset is reported
    /// in: scrolling to `0` rests the surface at its top.
    #[test]
    fn controller_target_resolves_to_the_raw_offset_the_reporter_inverts() {
        let inset = toolbar_inset();
        let target = Point::new(0.0, 300.0);
        let raw = raw_content_offset(target, inset);
        assert_eq!(raw, cocoa_ui::Point::new(0.0, 248.0));
        assert_eq!(inset_relative_offset(raw, inset), target);
        assert_eq!(
            raw_content_offset(Point::zero(), inset),
            cocoa_ui::Point::new(0.0, -52.0)
        );
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
            NO_SCROLLERS,
            paragraph,
        );
        assert_eq!(size, Size::new(48.0, 600.0));
    }

    #[test]
    fn vertical_scroll_min_height_is_zero() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(402.0), Some(0.0)),
            NO_SCROLLERS,
            paragraph,
        );
        assert_eq!(size, Size::new(402.0, 0.0));
    }

    #[test]
    fn horizontal_scroll_min_height_is_the_contents_minimum() {
        let size = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(Some(402.0), Some(0.0)),
            NO_SCROLLERS,
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
            NO_SCROLLERS,
            paragraph,
        );
        assert_eq!(size, Size::new(2200.0, 80.0));

        let twin = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(None, Some(80.0)),
            NO_SCROLLERS,
            |_| Size::new(60.0, 300.0),
        );
        assert_eq!(twin, Size::new(60.0, 80.0));
    }

    /// A rigid 28 pt-tall chip row: whatever the height offer, it answers
    /// 28 — the shape that exposed water-rs/waterui#1208.
    fn chip_row(proposal: ProposalSize) -> Size {
        Size::new(proposal.width.unwrap_or(328.5), 28.0)
    }

    /// The issue scenario (water-rs/waterui#1208): a horizontal scroll of
    /// 28 pt-tall content under a 240x160 offer answers 240x28 — it hugs the
    /// content on the non-scrolling axis instead of claiming the slot.
    #[test]
    fn horizontal_scroll_hugs_the_contents_height() {
        let size = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(Some(240.0), Some(160.0)),
            NO_SCROLLERS,
            chip_row,
        );
        assert_eq!(size, Size::new(240.0, 28.0));
    }

    /// The symmetric case: a vertical scroll of 28 pt-wide content under a
    /// 240x160 offer answers 28x160.
    #[test]
    fn vertical_scroll_hugs_the_contents_width() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(240.0), Some(160.0)),
            NO_SCROLLERS,
            |proposal| Size::new(28.0, proposal.height.unwrap_or(328.5)),
        );
        assert_eq!(size, Size::new(28.0, 160.0));
    }

    /// The hug clamps to a finite offer: 28 pt content under a 10 pt offer
    /// answers 10, never more. `0` still answers the content's minimum and
    /// `None` its intrinsic extent.
    #[test]
    fn non_scrolling_answer_clamps_to_the_offer() {
        let clamped = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(Some(240.0), Some(10.0)),
            NO_SCROLLERS,
            chip_row,
        );
        assert_eq!(clamped, Size::new(240.0, 10.0));
        let floor = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(Some(240.0), Some(0.0)),
            NO_SCROLLERS,
            chip_row,
        );
        assert_eq!(floor, Size::new(240.0, 28.0));
        let intrinsic = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(Some(240.0), None),
            NO_SCROLLERS,
            chip_row,
        );
        assert_eq!(intrinsic, Size::new(240.0, 28.0));
        // The scrolling axis is unchanged: `None` asks for the content's
        // intrinsic extent.
        let unspecified = scroll_min_size(
            Axis::Horizontal,
            ProposalSize::new(None, Some(160.0)),
            NO_SCROLLERS,
            chip_row,
        );
        assert_eq!(unspecified, Size::new(328.5, 28.0));
    }

    /// Content that itself fills the non-scrolling axis still fills it —
    /// the paragraph wraps to the offered width, so a vertical scroll under
    /// a finite width fills it.
    #[test]
    fn filling_content_still_fills_the_non_scrolling_axis() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(240.0), Some(160.0)),
            NO_SCROLLERS,
            paragraph,
        );
        assert_eq!(size, Size::new(240.0, 160.0));
    }

    /// An unspecified scrolling-axis answer reports the content's extent at
    /// the offered non-scrolling extent — the wrapped height, not the
    /// unwrapped intrinsic the previous claim-everything rule also
    /// measured, but the height the layout pass will actually place.
    #[test]
    fn unspecified_scrolling_axis_answers_at_the_offered_cross_extent() {
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(240.0), None),
            NO_SCROLLERS,
            paragraph,
        );
        assert_eq!(size, Size::new(240.0, 200.0));
    }

    /// water-rs/waterui#3597: beside a legacy scroller the viewport, not
    /// the frame, holds the content's answer. A caption 535 pt wide hugs
    /// into a 552 pt scroll — `frameSizeForContentSize:` of 535 under a
    /// legacy scroller — and is offered the 783 pt viewport an 800 pt offer
    /// leaves, so it never wraps against the scroller.
    #[test]
    fn a_legacy_scroller_widens_the_hug_by_its_track() {
        let offered = core::cell::Cell::new(None);
        let size = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(800.0), Some(600.0)),
            LEGACY_VERTICAL_SCROLLER,
            |proposal| {
                offered.set(proposal.width);
                Size::new(
                    proposal.width.map_or(535.0, |width| width.min(535.0)),
                    900.0,
                )
            },
        );
        assert_eq!(size, Size::new(552.0, 600.0));
        assert_eq!(offered.get(), Some(783.0));
        // Filling content still fills the offer, scroller included.
        let filled = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(Some(240.0), Some(160.0)),
            LEGACY_VERTICAL_SCROLLER,
            paragraph,
        );
        assert_eq!(filled, Size::new(240.0, 160.0));
        // The unspecified width is the content's ideal plus the track.
        let ideal = scroll_min_size(
            Axis::Vertical,
            ProposalSize::new(None, Some(160.0)),
            LEGACY_VERTICAL_SCROLLER,
            paragraph,
        );
        assert_eq!(ideal, Size::new(2217.0, 160.0));
    }

    /// `Axis::All` is unchanged: it fills a fully finite offer.
    #[test]
    fn bidirectional_scroll_fills_the_offer() {
        let size = scroll_min_size(
            Axis::All,
            ProposalSize::new(Some(240.0), Some(160.0)),
            NO_SCROLLERS,
            chip_row,
        );
        assert_eq!(size, Size::new(240.0, 160.0));
    }
}
