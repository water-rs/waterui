//! Reactive frame constraints for overriding a child's incoming proposal.

use alloc::{collections::BTreeSet, vec, vec::Vec};
use nami::{Computed, Signal, SignalExt};
use waterui_core::{
    AnyView, IntoSignalF32, View,
    layout::{LayoutInvalidationCallback, StretchAxis},
};

use crate::{
    Layout, PlacedSubview, Point, ProposalSize, Rect, Size, SubView, ViewDimensions,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Layout that clamps a single child's proposal to reactive frame constraints.
#[derive(Debug, Clone, Default)]
pub struct FrameLayout {
    min_width: Option<Computed<f32>>,
    ideal_width: Option<Computed<f32>>,
    max_width: Option<Computed<f32>>,
    min_height: Option<Computed<f32>>,
    ideal_height: Option<Computed<f32>>,
    max_height: Option<Computed<f32>>,
    alignment: Alignment,
}

#[derive(Clone, Copy)]
struct ResolvedFrameLayout {
    min_width: Option<f32>,
    ideal_width: Option<f32>,
    max_width: Option<f32>,
    min_height: Option<f32>,
    ideal_height: Option<f32>,
    max_height: Option<f32>,
}

impl FrameLayout {
    fn resolved(&self) -> ResolvedFrameLayout {
        ResolvedFrameLayout {
            min_width: self.min_width.as_ref().map(Signal::get),
            ideal_width: self.ideal_width.as_ref().map(Signal::get),
            max_width: self.max_width.as_ref().map(Signal::get),
            min_height: self.min_height.as_ref().map(Signal::get),
            ideal_height: self.ideal_height.as_ref().map(Signal::get),
            max_height: self.max_height.as_ref().map(Signal::get),
        }
    }
}

impl Layout for FrameLayout {
    /// A frame whose `max` on an axis is infinite is greedy on that axis —
    /// `SwiftUI`'s `.frame(maxWidth: .infinity)`. With stacks content-sized,
    /// this (plus `Spacer`/`Color`) is how a view opts into filling its
    /// container.
    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        let resolved = self.resolved();
        let horizontal = resolved.max_width.is_some_and(f32::is_infinite);
        let vertical = resolved.max_height.is_some_and(f32::is_infinite);
        match (horizontal, vertical) {
            (true, true) => StretchAxis::Both,
            (true, false) => StretchAxis::Horizontal,
            (false, true) => StretchAxis::Vertical,
            (false, false) => StretchAxis::None,
        }
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let resolved = self.resolved();
        // A Frame proposes a modified size to its single child. The parent's
        // proposal is what the child hears; an ideal only fills in a dimension
        // the parent left unspecified. Either way the frame's own min/max limit
        // what it passes down.
        let child_proposal = ProposalSize {
            width: frame_child_proposal_axis(
                proposal.width,
                resolved.min_width,
                resolved.ideal_width,
                resolved.max_width,
            ),
            height: frame_child_proposal_axis(
                proposal.height,
                resolved.min_height,
                resolved.ideal_height,
                resolved.max_height,
            ),
        };

        // Measure the child with our constrained proposal
        let child_dimensions = children
            .first()
            .map_or(ViewDimensions::new(Size::zero()), |c| {
                c.measure(child_proposal)
            });
        let child_size = child_dimensions.size;

        // Resolve the frame size on each axis. With no bound on an axis the
        // frame is exactly as big as the child it just measured; a maximum is
        // something to grow into, so there the frame takes the extent it was
        // offered instead. `place` proposes that resolved extent back to the
        // child, so the child is measured at the extent it will be placed in.
        let final_width = frame_resolved_axis(
            proposal.width,
            child_size.width,
            resolved.min_width,
            resolved.ideal_width,
            resolved.max_width,
        );
        let final_height = frame_resolved_axis(
            proposal.height,
            child_size.height,
            resolved.min_height,
            resolved.ideal_height,
            resolved.max_height,
        );

        Size::new(final_width, final_height)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        let resolved = self.resolved();
        // The frame's own extent is already settled by the time it is placed:
        // `size_that_fits` resolved it and the parent handed it back as
        // `bounds`. The child is therefore proposed that resolved extent, which
        // is the extent it was measured at — a resizable child already reported
        // it, and a rigid one reports the same natural size either way. The
        // ideal gets no second say: it answers an unspecified proposal, it is
        // not a cap on the child, so a frame that grew past its ideal
        // (`.frame(idealWidth: 24, maxWidth: .infinity)`, or a parent that
        // stretched it) has a child that fills it. Min/max still bind here
        // because the frame keeps its own constraints even when a parent offers
        // bounds that violate them.
        let child_proposal = ProposalSize {
            width: Some(clamp_frame_axis(
                bounds.width(),
                resolved.min_width,
                resolved.max_width,
            )),
            height: Some(clamp_frame_axis(
                bounds.height(),
                resolved.min_height,
                resolved.max_height,
            )),
        };

        let child_dimensions = children
            .first()
            .map_or(ViewDimensions::new(Size::zero()), |c| {
                c.measure(child_proposal)
            });
        let child_size = child_dimensions.size;

        // Handle infinite dimensions (axis-expanding views)
        let child_width = if child_size.width.is_infinite() {
            bounds.width()
        } else {
            child_size.width
        };

        let child_height = if child_size.height.is_infinite() {
            bounds.height()
        } else {
            child_size.height
        };

        let final_child_size = Size::new(child_width, child_height);
        let mut adjusted_dimensions = child_dimensions;
        adjusted_dimensions.size = final_child_size;

        // Calculate the child's origin point (top-left) based on alignment.
        let horizontal = self.alignment.horizontal();
        let horizontal_target = if horizontal == HorizontalAlignment::Leading {
            0.0
        } else if horizontal == HorizontalAlignment::Trailing {
            bounds.width()
        } else if horizontal == HorizontalAlignment::Center {
            bounds.width() * 0.5
        } else {
            adjusted_dimensions
                .horizontal(horizontal)
                .clamp(0.0, final_child_size.width)
        };
        let child_x = bounds.x() + horizontal_target
            - adjusted_dimensions
                .horizontal(horizontal)
                .clamp(0.0, final_child_size.width);

        let vertical = self.alignment.vertical();
        let vertical_target = if vertical == VerticalAlignment::Top {
            0.0
        } else if vertical == VerticalAlignment::Bottom {
            bounds.height()
        } else if vertical == VerticalAlignment::Center {
            bounds.height() * 0.5
        } else {
            adjusted_dimensions
                .vertical(vertical)
                .clamp(0.0, final_child_size.height)
        };
        let child_y = bounds.y() + vertical_target
            - adjusted_dimensions
                .vertical(vertical)
                .clamp(0.0, final_child_size.height);

        vec![Rect::new(Point::new(child_x, child_y), final_child_size)]
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .first()
            .and_then(|child| child.explicit_horizontal(alignment))
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .first()
            .and_then(|child| child.explicit_vertical(alignment))
    }

    fn watch_invalidation(
        &self,
        invalidate: LayoutInvalidationCallback,
    ) -> Vec<nami::watcher::BoxWatcherGuard> {
        let signals = [
            self.min_width.as_ref(),
            self.ideal_width.as_ref(),
            self.max_width.as_ref(),
            self.min_height.as_ref(),
            self.ideal_height.as_ref(),
            self.max_height.as_ref(),
        ];
        let mut identities = BTreeSet::new();
        let mut guards = Vec::new();
        for signal in signals.into_iter().flatten() {
            if signal
                .identity()
                .is_some_and(|identity| !identities.insert(identity))
            {
                continue;
            }
            let invalidate = invalidate.clone();
            guards.push(signal.watch(move |_| invalidate()));
        }
        guards
    }
}

#[inline]
fn clamp_frame_axis(value: f32, min: Option<f32>, max: Option<f32>) -> f32 {
    value
        .max(min.unwrap_or(f32::NEG_INFINITY))
        .min(max.unwrap_or(f32::INFINITY))
}

/// The proposal a frame hands its child on one axis.
///
/// `SwiftUI`'s flexible frame proposes the size proposed to the frame, limited
/// by any constraints, with any ideal dimensions replacing the *unspecified*
/// dimensions of that proposal. So a parent that has a size in mind is the one
/// the child hears, the ideal answers only the axes nobody had an opinion on,
/// and the frame's own `min`/`max` bind whichever of the two came through.
///
/// An ideal that outranked the proposal would be a cap on the child, and a cap
/// is what `max` is for: it would leave a resizable drawing unable to fill the
/// frame it had already grown into.
#[inline]
fn frame_child_proposal_axis(
    parent_proposal: Option<f32>,
    min: Option<f32>,
    ideal: Option<f32>,
    max: Option<f32>,
) -> Option<f32> {
    parent_proposal
        .or(ideal)
        .map(|value| clamp_frame_axis(value, min, max))
}

/// Resolves the frame's own extent on one axis.
///
/// A frame grows into the space offered *up to* the maximum it was given, which
/// is how `.frame(maxWidth: .infinity)` fills and `.frame(maxWidth: 100)` fills
/// to a hundred points. The extent it grows into is the one it was offered —
/// the parent's proposal, or the ideal when the parent proposed nothing.
///
/// Without a maximum there is nothing to grow into, so the frame adopts its
/// child's sizing behaviour on that axis: it is exactly as big as the child it
/// measured, clamped up by any minimum. That is what keeps `.frame(minHeight:
/// 44)` on a label a 44pt-tall label rather than one stretched over whatever
/// height the parent happened to propose.
///
/// It is also why an *ideal* is an answer to an unspecified proposal rather
/// than a pin. The ideal reaches the child through
/// [`frame_child_proposal_axis`] and comes back as the child's own answer: a
/// child that takes what it is offered reports the ideal, a rigid one reports
/// its natural size, and neither is overridden here. `min`/`max` are how a
/// caller asks for a pin.
#[inline]
fn frame_resolved_axis(
    parent_proposal: Option<f32>,
    child_size: f32,
    min: Option<f32>,
    ideal: Option<f32>,
    max: Option<f32>,
) -> f32 {
    let content = if max.is_some() {
        parent_proposal.or(ideal).unwrap_or(child_size)
    } else {
        child_size
    };
    clamp_frame_axis(content, min, max)
}

/// A view that provides a frame with optional size constraints and alignment for its child.
///
/// The Frame view allows you to specify minimum, ideal, and maximum dimensions
/// for width and height, and controls how the child is aligned within the frame.
#[derive(Debug)]
pub struct Frame {
    layout: FrameLayout,
    content: AnyView,
}

impl Frame {
    /// Creates a new Frame with the specified content and alignment.
    ///
    /// # Arguments
    /// * `content` - The child view to be contained within the frame
    /// * `alignment` - How the child should be aligned within the frame
    #[must_use]
    pub fn new(content: impl View) -> Self {
        Self {
            layout: FrameLayout::default(),
            content: AnyView::new(content),
        }
    }

    /// Sets the alignment of the child within the frame.
    ///
    /// # Arguments
    /// * `alignment` - The alignment to apply to the child view
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }

    /// Sets the ideal width of the frame.
    ///
    /// Accepts any numeric literal or signal of f32 (`f32`, `f64`, `i32`,
    /// `Computed<f32>`, `Binding<f32>`, …). Signal changes invalidate only this
    /// frame's native layout.
    #[must_use]
    pub fn width(mut self, width: impl IntoSignalF32 + 'static) -> Self {
        let width = width.into_signal_f32().computed();
        self.layout.min_width = Some(width.clone());
        self.layout.ideal_width = Some(width.clone());
        self.layout.max_width = Some(width);
        self
    }

    /// Sets the ideal height of the frame.
    #[must_use]
    pub fn height(mut self, height: impl IntoSignalF32 + 'static) -> Self {
        let height = height.into_signal_f32().computed();
        self.layout.min_height = Some(height.clone());
        self.layout.ideal_height = Some(height.clone());
        self.layout.max_height = Some(height);
        self
    }

    /// Sets the width the frame asks for when its parent proposes nothing.
    ///
    /// This is `SwiftUI`'s `.frame(idealWidth:)`. Unlike [`width`](Self::width),
    /// which pins all three constraints, it leaves the frame free to be smaller
    /// or larger when the parent does have a size in mind.
    #[must_use]
    pub fn ideal_width(mut self, width: impl IntoSignalF32 + 'static) -> Self {
        self.layout.ideal_width = Some(width.into_signal_f32().computed());
        self
    }

    /// Sets the height the frame asks for when its parent proposes nothing.
    ///
    /// This is `SwiftUI`'s `.frame(idealHeight:)`; see [`ideal_width`](Self::ideal_width).
    #[must_use]
    pub fn ideal_height(mut self, height: impl IntoSignalF32 + 'static) -> Self {
        self.layout.ideal_height = Some(height.into_signal_f32().computed());
        self
    }

    /// Sets the minimum width of the frame.
    #[must_use]
    pub fn min_width(mut self, width: impl IntoSignalF32 + 'static) -> Self {
        self.layout.min_width = Some(width.into_signal_f32().computed());
        self
    }

    /// Sets the maximum width of the frame.
    #[must_use]
    pub fn max_width(mut self, width: impl IntoSignalF32 + 'static) -> Self {
        self.layout.max_width = Some(width.into_signal_f32().computed());
        self
    }

    /// Sets the minimum height of the frame.
    #[must_use]
    pub fn min_height(mut self, height: impl IntoSignalF32 + 'static) -> Self {
        self.layout.min_height = Some(height.into_signal_f32().computed());
        self
    }

    /// Sets the maximum height of the frame.
    #[must_use]
    pub fn max_height(mut self, height: impl IntoSignalF32 + 'static) -> Self {
        self.layout.max_height = Some(height.into_signal_f32().computed());
        self
    }
}

impl View for Frame {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        // The Frame view's body is just a Container with our custom layout and the child content.
        FixedContainer::new(self.layout, vec![self.content])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StretchAxis;
    use alloc::rc::Rc;
    use core::cell::Cell;
    use nami::{SignalExt, binding};

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    /// A greedy child: it takes whatever extent it is offered, the way a
    /// `Color` or a fill-scaled image does, and falls back to its intrinsic
    /// size on an axis nobody proposed.
    struct FillingSubView {
        intrinsic: Size,
    }

    impl SubView for FillingSubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(Size::new(
                proposal.width.unwrap_or(self.intrinsic.width),
                proposal.height.unwrap_or(self.intrinsic.height),
            ))
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::Both
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    /// Wraps a child so a test can see the proposal the frame handed it.
    struct RecordingSubView<C> {
        inner: C,
        proposal: Cell<Option<ProposalSize>>,
    }

    impl<C> RecordingSubView<C> {
        fn new(inner: C) -> Self {
            Self {
                inner,
                proposal: Cell::new(None),
            }
        }

        fn proposal(&self) -> ProposalSize {
            self.proposal
                .get()
                .expect("the frame never measured its child")
        }
    }

    impl<C: SubView> SubView for RecordingSubView<C> {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            self.proposal.set(Some(proposal));
            self.inner.measure(proposal)
        }
        fn stretch_axis(&self) -> StretchAxis {
            self.inner.stretch_axis()
        }
        fn priority(&self) -> i32 {
            self.inner.priority()
        }
    }

    fn assert_extent(actual: f32, expected: f32, what: &str) {
        assert!(
            (actual - expected).abs() < f32::EPSILON,
            "expected {what} to be {expected}, got {actual}"
        );
    }

    #[test]
    fn an_ideal_frame_still_adopts_a_rigid_childs_size() {
        // `.frame(idealWidth: 100, idealHeight: 50)` with nothing proposed. The
        // ideal is what the child is asked for, but with neither a minimum nor
        // a maximum the frame has no size of its own — it is whatever the child
        // answered, and a rigid 30x20 drawing answers 30x20.
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(100.0)),
            ideal_height: Some(Computed::constant(50.0)),
            ..Default::default()
        };

        let child = RecordingSubView::new(MockSubView {
            size: Size::new(30.0, 20.0),
        });

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);

        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(100.0), Some(50.0)),
            "the ideal should answer the dimensions the parent left unspecified"
        );
        assert_extent(size.width, 30.0, "the frame's width");
        assert_extent(size.height, 20.0, "the frame's height");
    }

    #[test]
    fn an_ideal_answers_only_the_dimensions_the_parent_left_unspecified() {
        // `.frame(idealWidth: 24, idealHeight: 24)` under a 48x36 proposal. The
        // parent has an opinion on both dimensions, so the ideal has nothing to
        // say: the child hears 48x36. With neither a minimum nor a maximum the
        // frame then adopts its child's sizing behaviour — a resizable drawing
        // takes the whole 48x36, a rigid one stays at its own 24x24 — which is
        // what `SwiftUI` gives for the same frame.
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(24.0)),
            ideal_height: Some(Computed::constant(24.0)),
            ..Default::default()
        };
        let proposal = ProposalSize::new(Some(48.0), Some(36.0));

        let resizable = RecordingSubView::new(FillingSubView {
            intrinsic: Size::new(24.0, 24.0),
        });
        let filled = layout.size_that_fits(proposal, &[&resizable]);
        assert_eq!(
            resizable.proposal(),
            proposal,
            "the child should hear the parent's proposal, not the ideal"
        );
        assert_extent(filled.width, 48.0, "a resizable child's frame width");
        assert_extent(filled.height, 36.0, "a resizable child's frame height");

        // Placement agrees with measurement by construction: the frame proposes
        // the extent it resolved to, which is the extent the child was measured
        // at and reported back.
        let rects = layout.place(Rect::new(Point::new(0.0, 0.0), filled), &[&resizable]);
        assert_eq!(
            resizable.proposal(),
            proposal,
            "placement should re-propose the extent the child was measured at"
        );
        assert_extent(rects[0].width(), 48.0, "the resizable child's width");
        assert_extent(rects[0].height(), 36.0, "the resizable child's height");

        let rigid = RecordingSubView::new(MockSubView {
            size: Size::new(24.0, 24.0),
        });
        let hugged = layout.size_that_fits(proposal, &[&rigid]);
        assert_eq!(
            rigid.proposal(),
            proposal,
            "a rigid child hears the proposal too; it just declines it"
        );
        assert_extent(hugged.width, 24.0, "a rigid child's frame width");
        assert_extent(hugged.height, 24.0, "a rigid child's frame height");
    }

    #[test]
    fn a_rigid_child_does_not_grow_into_a_larger_proposal() {
        // The same frame around a rigid 24pt drawing offered 400pt: the drawing
        // declines, and with no maximum the frame has nothing to grow into. A
        // resizable child would take the 400 — see the test above.
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(24.0)),
            ..Default::default()
        };

        let child = MockSubView {
            size: Size::new(24.0, 24.0),
        };

        let size = layout.size_that_fits(ProposalSize::new(Some(400.0), None), &[&child]);

        assert_extent(size.width, 24.0, "the frame's width");
    }

    #[test]
    fn test_frame_alignment() {
        let layout = FrameLayout {
            alignment: Alignment::BottomTrailing,
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &children);

        // Child should be at bottom-trailing corner
        assert!((rects[0].x() - 70.0).abs() < f32::EPSILON); // 100 - 30
        assert!((rects[0].y() - 80.0).abs() < f32::EPSILON); // 100 - 20
    }

    #[test]
    fn test_fixed_width_resists_zero_min_query() {
        let layout = FrameLayout {
            min_width: Some(Computed::constant(120.0)),
            ideal_width: Some(Computed::constant(120.0)),
            max_width: Some(Computed::constant(120.0)),
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::ZERO, &children);
        assert!((size.width - 120.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_min_width_clamps_zero_min_query() {
        let layout = FrameLayout {
            min_width: Some(Computed::constant(64.0)),
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(10.0, 10.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::ZERO, &children);
        assert!((size.width - 64.0).abs() < f32::EPSILON);
    }

    #[test]
    fn reactive_width_invalidates_and_updates_measurement() {
        let width = binding(80.0_f32);
        let signal = width.computed();
        let layout = FrameLayout {
            min_width: Some(signal.clone()),
            ideal_width: Some(signal.clone()),
            max_width: Some(signal),
            ..Default::default()
        };
        let invalidations = Rc::new(Cell::new(0));
        let callback_invalidations = Rc::clone(&invalidations);
        let _guards = layout.watch_invalidation(Rc::new(move || {
            callback_invalidations.set(callback_invalidations.get() + 1);
        }));

        width.set(120.0);
        assert_eq!(invalidations.get(), 1);

        let child = MockSubView {
            size: Size::new(10.0, 10.0),
        };
        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
        assert_eq!(size.width, 120.0);
    }

    #[test]
    fn a_minimum_alone_does_not_make_the_frame_greedy() {
        // `.frame(minWidth: 50)` on a 30pt child inside a 300pt parent is a 50pt
        // frame, not a 300pt one: with no maximum there is nothing to grow into.
        let layout = FrameLayout {
            min_width: Some(Computed::constant(50.0)),
            ..Default::default()
        };

        let child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let size = layout.size_that_fits(ProposalSize::new(Some(300.0), None), &[&child]);

        assert!(
            (size.width - 50.0).abs() < f32::EPSILON,
            "expected the frame to stay at its minimum, got {}",
            size.width
        );
    }

    #[test]
    fn a_bounded_maximum_fills_up_to_that_maximum() {
        // `.frame(maxWidth: 100)` does grow into the offer, stopping at 100.
        let layout = FrameLayout {
            max_width: Some(Computed::constant(100.0)),
            ..Default::default()
        };

        let child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let size = layout.size_that_fits(ProposalSize::new(Some(300.0), None), &[&child]);

        assert!(
            (size.width - 100.0).abs() < f32::EPSILON,
            "expected the frame to fill to its maximum, got {}",
            size.width
        );
    }

    #[test]
    fn an_infinite_maximum_takes_the_whole_offer() {
        let layout = FrameLayout {
            max_width: Some(Computed::constant(f32::INFINITY)),
            ..Default::default()
        };

        let child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let size = layout.size_that_fits(ProposalSize::new(Some(300.0), None), &[&child]);

        assert!(
            (size.width - 300.0).abs() < f32::EPSILON,
            "expected the frame to fill the proposal, got {}",
            size.width
        );
    }

    #[test]
    fn an_ideal_size_is_what_an_unconstrained_parent_gets() {
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(120.0)),
            ..Default::default()
        };

        let child = FillingSubView {
            intrinsic: Size::new(30.0, 20.0),
        };

        // Nothing proposed: the ideal is what the child is asked for, and a
        // child that takes what it is offered hands the ideal back.
        let unconstrained = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
        assert_extent(
            unconstrained.width,
            120.0,
            "the unconstrained frame's width",
        );

        // A parent that does have a size in mind is heard instead: the ideal
        // replaces unspecified dimensions only, and it is no more a cap on the
        // child than it is a pin, so a resizable child fills the 300 offered.
        let proposed = layout.size_that_fits(ProposalSize::new(Some(300.0), Some(90.0)), &[&child]);
        assert_extent(proposed.width, 300.0, "the proposed frame's width");
        assert_extent(proposed.height, 90.0, "the proposed frame's height");
    }

    #[test]
    fn an_ideal_does_not_cap_the_child_once_the_frame_has_grown() {
        // `.frame(idealWidth: 100, idealHeight: 50, maxWidth: .infinity,
        // maxHeight: .infinity)` in a 320x180 slot. The ideal answered the
        // sizing question; the maximum then let the frame fill, and the child
        // is proposed the frame it actually got, not the ideal.
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(100.0)),
            ideal_height: Some(Computed::constant(50.0)),
            max_width: Some(Computed::constant(f32::INFINITY)),
            max_height: Some(Computed::constant(f32::INFINITY)),
            ..Default::default()
        };

        let child = RecordingSubView::new(FillingSubView {
            intrinsic: Size::new(30.0, 20.0),
        });

        let size = layout.size_that_fits(ProposalSize::new(Some(320.0), Some(180.0)), &[&child]);
        assert_extent(size.width, 320.0, "the frame's width");
        assert_extent(size.height, 180.0, "the frame's height");

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(320.0, 180.0));
        let rects = layout.place(bounds, &[&child]);

        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(320.0), Some(180.0)),
            "the child should be proposed the frame's resolved bounds"
        );
        assert_extent(rects[0].width(), 320.0, "the child's width");
        assert_extent(rects[0].height(), 180.0, "the child's height");
    }

    #[test]
    fn an_unconstrained_frame_takes_its_ideal_and_hands_that_to_the_child() {
        // Nobody proposes anything, so the ideal is the frame — and the child
        // hears the ideal because that is what the frame resolved to.
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(100.0)),
            ideal_height: Some(Computed::constant(50.0)),
            ..Default::default()
        };

        let child = RecordingSubView::new(FillingSubView {
            intrinsic: Size::new(30.0, 20.0),
        });

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
        assert_extent(size.width, 100.0, "the frame's width");
        assert_extent(size.height, 50.0, "the frame's height");

        let rects = layout.place(Rect::new(Point::new(0.0, 0.0), size), &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(100.0), Some(50.0)),
            "the child should be proposed the ideal the frame resolved to"
        );
        assert_extent(rects[0].width(), 100.0, "the child's width");
        assert_extent(rects[0].height(), 50.0, "the child's height");
    }

    #[test]
    fn placement_holds_the_frames_own_minimum_and_maximum() {
        // `.frame(minWidth: 120, maxHeight: 40)` handed an 80x300 slot: the
        // frame's own constraints still bind on the proposal it passes down,
        // so the child hears 120 wide and 40 tall rather than the raw bounds.
        let layout = FrameLayout {
            min_width: Some(Computed::constant(120.0)),
            max_height: Some(Computed::constant(40.0)),
            ..Default::default()
        };

        let child = RecordingSubView::new(FillingSubView {
            intrinsic: Size::new(30.0, 20.0),
        });

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(80.0, 300.0));
        layout.place(bounds, &[&child]);

        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(120.0), Some(40.0)),
            "the child's proposal should be the bounds clamped by the frame"
        );
    }

    #[test]
    fn an_ideal_that_is_also_the_maximum_is_a_natural_size_that_only_shrinks() {
        // The shape `waterui-svg` carries an icon's intrinsic size in:
        // `.frame(idealWidth: 24, idealHeight: 24, maxWidth: 24, maxHeight: 24)`
        // around a scene that takes whatever it is proposed. The ideal answers
        // the axes nobody proposed, the maximum keeps the drawing from growing
        // past its natural size, and the absence of a minimum leaves it free to
        // shrink into a smaller box.
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(24.0)),
            ideal_height: Some(Computed::constant(24.0)),
            max_width: Some(Computed::constant(24.0)),
            max_height: Some(Computed::constant(24.0)),
            ..Default::default()
        };
        let scene = || {
            RecordingSubView::new(FillingSubView {
                intrinsic: Size::zero(),
            })
        };

        // Nothing proposed: the natural size.
        let child = scene();
        let natural = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
        assert_extent(natural.width, 24.0, "the unproposed width");
        assert_extent(natural.height, 24.0, "the unproposed height");

        // A row that proposes its own height — `HStack` hands a non-stretching
        // child `(None, bounds.height())` — leaves the icon at 24, not 44.
        let child = scene();
        let in_a_row = layout.size_that_fits(ProposalSize::new(None, Some(44.0)), &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(24.0), Some(24.0)),
            "the scene should never be offered more than the natural size"
        );
        assert_extent(in_a_row.width, 24.0, "the width in a taller row");
        assert_extent(in_a_row.height, 24.0, "the height in a taller row");

        // `.size(8, 10)` still gets through: there is no minimum to stop it.
        let child = scene();
        let resized = layout.size_that_fits(ProposalSize::new(Some(8.0), Some(10.0)), &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(8.0), Some(10.0)),
            "a smaller box should reach the scene unaltered"
        );
        assert_extent(resized.width, 8.0, "the resized width");
        assert_extent(resized.height, 10.0, "the resized height");
    }

    #[test]
    fn a_filling_frame_offers_its_whole_width_to_a_rigid_child() {
        // `.frame(maxWidth: .infinity)` around a 30x20 icon in a 320x180 slot.
        // The frame fills the width and stays the icon's height; the icon is
        // offered all of that width, declines it, and is centred in it.
        let layout = FrameLayout {
            max_width: Some(Computed::constant(f32::INFINITY)),
            ..Default::default()
        };

        let child = RecordingSubView::new(MockSubView {
            size: Size::new(30.0, 20.0),
        });

        let size = layout.size_that_fits(ProposalSize::new(Some(320.0), Some(180.0)), &[&child]);
        assert_extent(size.width, 320.0, "the frame's width");
        assert_extent(size.height, 20.0, "the frame's height");

        let rects = layout.place(Rect::new(Point::new(0.0, 0.0), size), &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(320.0), Some(20.0)),
            "a rigid child is still offered the frame it sits in"
        );
        assert_extent(rects[0].width(), 30.0, "the child's width");
        assert_extent(rects[0].x(), 145.0, "the child's x");
    }
}
