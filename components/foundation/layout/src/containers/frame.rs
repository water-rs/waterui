//! Reactive frame constraints for overriding a child's incoming proposal.

use alloc::{collections::BTreeSet, vec, vec::Vec};
use nami::{Computed, Signal, SignalExt};
use waterui_core::{
    AnyView, IntoSignalF32, View,
    layout::{LayoutInvalidationCallback, StretchAxis},
};

use crate::{
    Layout, PlacedSubview, Point, ProposalSize, Rect, Size, SubView, SubviewPlacement,
    ViewDimensions,
    container::FixedContainer,
    stack::{
        Alignment, HorizontalAlignment, VerticalAlignment,
        distribute::{LineAnchor, container_line},
    },
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

impl ResolvedFrameLayout {
    /// The frame's own extent for a child answer under `proposal`.
    fn resolve(&self, proposal: ProposalSize, child: Size) -> Size {
        Size::new(
            frame_resolved_axis(
                proposal.width,
                child.width,
                self.min_width,
                self.ideal_width,
                self.max_width,
            ),
            frame_resolved_axis(
                proposal.height,
                child.height,
                self.min_height,
                self.ideal_height,
                self.max_height,
            ),
        )
    }

    /// Placement negotiates on both axes against the bounds the frame was
    /// resolved to: every axis — constrained or not — re-proposes the
    /// resolved extent, still bound by the frame's own `min`/`max`.
    fn placement_proposal(&self, bounds: Rect) -> ProposalSize {
        self.child_proposal(ProposalSize {
            width: Some(bounds.width()),
            height: Some(bounds.height()),
        })
    }

    /// The proposal the frame hands its child for a given incoming proposal:
    /// the parent's offer on each axis, answered by the ideal where the parent
    /// proposed nothing and bound by the frame's own `min`/`max`.
    fn child_proposal(&self, proposal: ProposalSize) -> ProposalSize {
        ProposalSize {
            width: frame_child_proposal_axis(
                proposal.width,
                self.min_width,
                self.ideal_width,
                self.max_width,
            ),
            height: frame_child_proposal_axis(
                proposal.height,
                self.min_height,
                self.ideal_height,
                self.max_height,
            ),
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
        let child_proposal = resolved.child_proposal(proposal);
        let Some(child) = children.first() else {
            return resolved.resolve(proposal, Size::zero());
        };

        // Resolve the frame size on each axis. With no bound on an axis the
        // frame is exactly as big as the child it just measured; a maximum is
        // something to grow into, so there the frame takes the extent it was
        // offered instead.
        let first = resolved.resolve(proposal, child.measure(child_proposal).size);

        // `place` proposes the resolved bounds back to the child on every
        // axis, and the child's answer can depend on it (text wraps to the
        // width it is given). The frame's answer is therefore taken from the
        // child measured under the proposal it will be placed with, so
        // measurement and placement agree.
        let placement = resolved.placement_proposal(Rect::from_size(first));
        if placement == child_proposal {
            first
        } else {
            resolved.resolve(proposal, child.measure(placement).size)
        }
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        if children.is_empty() {
            return vec![];
        }

        let resolved = self.resolved();
        // Placement is a fresh negotiation against the bounds: the child is
        // re-proposed the resolved extent on every axis.
        let child_proposal = resolved.placement_proposal(bounds);

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

        // The child sits with its guide on the frame's alignment line,
        // wherever that guide lies; a child larger than the frame overflows.
        let horizontal = self.alignment.horizontal();
        let guide_x = adjusted_dimensions.horizontal(horizontal);
        let child_x = bounds.x()
            + container_line(
                bounds.width(),
                LineAnchor::horizontal(horizontal),
                guide_x,
                final_child_size.width - guide_x,
            )
            - guide_x;

        let vertical = self.alignment.vertical();
        let guide_y = adjusted_dimensions.vertical(vertical);
        let child_y = bounds.y()
            + container_line(
                bounds.height(),
                LineAnchor::vertical(vertical),
                guide_y,
                final_child_size.height - guide_y,
            )
            - guide_y;

        vec![SubviewPlacement::new(
            Rect::new(Point::new(child_x, child_y), final_child_size),
            child_proposal,
        )]
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
/// A zero proposal is not an offer, though: it is the min-size query, and the
/// frame's floor is its child's answer, clamped by the frame's own
/// constraints. A frame that claims it can shrink to nothing while the content
/// it aligns needs room reports a minimum it cannot honour — and a stack that
/// trusts that minimum starves the slot while the child is still laid out at
/// its full extent, centred over the frame's surroundings.
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
    let content = if max.is_some() && parent_proposal != Some(0.0) {
        let extent = parent_proposal.or(ideal).unwrap_or(child_size);
        // An unbounded answer is legal only on an axis the child left
        // unbounded: a maximum is something to grow into, not a source of
        // infinity of its own.
        if extent.is_infinite() && child_size.is_finite() {
            child_size
        } else {
            extent
        }
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
    /// * `alignment` - The alignment to apply to the child view: an
    ///   [`Alignment`] or one of the tokens (`Leading`, `TopTrailing`,
    ///   `Center`, …).
    #[must_use]
    pub fn alignment(mut self, alignment: impl Into<Alignment>) -> Self {
        self.layout.alignment = alignment.into();
        self
    }

    crate::alignment::two_dimensional_alignment_methods!();

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

    /// Resolves to `FixedContainer` over the same layout and single child;
    /// reports what that container would.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&[self.content.stretch_axis()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StretchAxis;
    use alloc::rc::Rc;
    use core::cell::{Cell, RefCell};
    use nami::{SignalExt, binding};

    #[test]
    fn constrained_axes_select_their_owned_region_during_placement() {
        for (minimum, maximum, ideal, expected) in [
            (Some(28.0), Some(28.0), 0.0, 28.0),
            (Some(100.0), None, 0.0, 100.0),
            (None, Some(60.0), 200.0, 60.0),
        ] {
            let layout = FrameLayout {
                min_width: minimum.map(Computed::constant),
                max_width: maximum.map(Computed::constant),
                ..Default::default()
            };
            let child = RecordingSubView::new(FillingSubView {
                intrinsic: Size::new(ideal, 10.0),
            });
            let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
            assert_eq!(size, Size::new(expected, 10.0));
            let placements =
                layout.place(Rect::from_size(size), ProposalSize::UNSPECIFIED, &[&child]);
            assert_eq!(
                placements[0].proposal,
                ProposalSize::new(Some(expected), Some(10.0))
            );
            assert_eq!(*placements[0].frame.size(), Size::new(expected, 10.0));
        }
    }

    struct FramedChild<C> {
        layout: FrameLayout,
        child: C,
    }

    impl<C: SubView> SubView for FramedChild<C> {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.layout.size_that_fits(proposal, &[&self.child]))
        }
        fn stretch_axis(&self) -> StretchAxis {
            self.layout.stretch_axis(&[self.child.stretch_axis()])
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    struct WrappingChild;

    impl SubView for WrappingChild {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            let width = proposal.width.unwrap_or(200.0).clamp(10.0, 200.0);
            ViewDimensions::new(Size::new(width, (200.0 / width).ceil() * 10.0))
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn finite_stack_offers_remeasure_capped_wrapping_children() {
        let child = FramedChild {
            layout: FrameLayout {
                max_width: Some(Computed::constant(60.0)),
                ..Default::default()
            },
            child: WrappingChild,
        };
        let stack = crate::stack::HStackLayout::default();
        assert_eq!(
            stack.size_that_fits(ProposalSize::new(Some(240.0), None), &[&child]),
            Size::new(60.0, 40.0)
        );
    }

    #[test]
    fn finite_maximum_frames_grow_only_to_their_reported_limit() {
        let child = FramedChild {
            layout: FrameLayout {
                max_width: Some(Computed::constant(100.0)),
                ..Default::default()
            },
            child: FillingSubView {
                intrinsic: Size::new(20.0, 10.0),
            },
        };
        let stack = crate::stack::HStackLayout::default();
        for (offer, width) in [(80.0, 80.0), (240.0, 100.0)] {
            assert_eq!(
                stack.size_that_fits(ProposalSize::new(Some(offer), None), &[&child]),
                Size::new(width, 10.0)
            );
        }
    }

    #[test]
    fn a_max_only_frame_reports_the_child_floor_on_a_min_size_query() {
        // `.frame(maxWidth: .infinity)` around a rigid 220pt child: a zero
        // proposal is the min-size query, and the frame's floor is the child's
        // answer, not the bare offer. Reporting 0 lets a stack starve the slot
        // while the frame still centres the child's full extent over its
        // surroundings.
        let layout = FrameLayout {
            max_width: Some(Computed::constant(f32::INFINITY)),
            ..Default::default()
        };
        let child = MockSubView {
            size: Size::new(220.0, 40.0),
        };

        let size = layout.size_that_fits(ProposalSize::ZERO, &[&child]);
        assert_extent(size.width, 220.0, "the frame's minimum width");

        // A real offer still grows into it.
        let size = layout.size_that_fits(ProposalSize::new(Some(500.0), None), &[&child]);
        assert_extent(size.width, 500.0, "the frame's offered width");
    }

    #[test]
    fn a_bounded_maximum_clamps_the_reported_floor() {
        // `.frame(maxWidth: 100)` over a 220pt child: the reported floor is the
        // child's answer limited by the frame's own maximum, neither the offer
        // nor the child's whole extent.
        let layout = FrameLayout {
            max_width: Some(Computed::constant(100.0)),
            ..Default::default()
        };
        let child = MockSubView {
            size: Size::new(220.0, 40.0),
        };

        let size = layout.size_that_fits(ProposalSize::ZERO, &[&child]);
        assert_extent(size.width, 100.0, "the frame's clamped minimum width");
    }

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

    /// Wraps a child so a test can see the proposals the frame handed it.
    struct RecordingSubView<C> {
        inner: C,
        proposals: RefCell<Vec<ProposalSize>>,
    }

    impl<C> RecordingSubView<C> {
        fn new(inner: C) -> Self {
            Self {
                inner,
                proposals: RefCell::new(Vec::new()),
            }
        }

        fn proposal(&self) -> ProposalSize {
            self.proposals
                .borrow()
                .last()
                .copied()
                .expect("the frame never measured its child")
        }

        fn proposals(&self) -> Vec<ProposalSize> {
            self.proposals.borrow().clone()
        }
    }

    impl<C: SubView> SubView for RecordingSubView<C> {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            self.proposals.borrow_mut().push(proposal);
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
    fn an_ideal_frame_still_adopts_a_rigid_child_size() {
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
            child.proposals().as_slice(),
            &[
                ProposalSize::new(Some(100.0), Some(50.0)),
                ProposalSize::new(Some(30.0), Some(20.0)),
            ],
            "the ideal answers the unspecified axes, then the child is re-probed with the resolved bounds"
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

        // Placement agrees with measurement by construction: the child is
        // proposed the same constrained proposal it was measured with.
        let placements = layout.place(
            Rect::new(Point::new(0.0, 0.0), filled),
            proposal,
            &[&resizable],
        );
        assert_eq!(
            resizable.proposal(),
            proposal,
            "placement should re-propose what measurement proposed"
        );
        assert_extent(
            placements[0].frame.width(),
            48.0,
            "the resizable child's width",
        );
        assert_extent(
            placements[0].frame.height(),
            36.0,
            "the resizable child's height",
        );

        let rigid = RecordingSubView::new(MockSubView {
            size: Size::new(24.0, 24.0),
        });
        let hugged = layout.size_that_fits(proposal, &[&rigid]);
        assert_eq!(
            rigid.proposals().as_slice(),
            &[proposal, ProposalSize::new(Some(24.0), Some(24.0)),],
            "a rigid child hears the proposal too; it just declines it — and placement re-probes with the resolved bounds"
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
    fn test_frame_alignment_bounded_proposal() {
        let layout = FrameLayout {
            alignment: Alignment::BottomTrailing,
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout.place(bounds, proposal, &children);

        // Child should be at bottom-trailing corner
        assert!((placements[0].frame.x() - 70.0).abs() < f32::EPSILON); // 100 - 30
        assert!((placements[0].frame.y() - 80.0).abs() < f32::EPSILON); // 100 - 20
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
    fn frame_max_probe_does_not_invent_infinity() {
        // A filling frame grows into a finite offer, but a maximum query
        // cannot manufacture an infinite answer out of a finite child: an
        // unbounded answer is only legal on an axis the child itself left
        // unbounded.
        for (layout, proposal, bounds, expected_frame) in [
            (
                FrameLayout {
                    max_width: Some(Computed::constant(f32::INFINITY)),
                    ..Default::default()
                },
                ProposalSize::new(Some(f32::INFINITY), Some(10.0)),
                Rect::from_size(Size::new(100.0, 10.0)),
                Rect::new(Point::new(40.0, 0.0), Size::new(20.0, 10.0)),
            ),
            (
                FrameLayout {
                    max_height: Some(Computed::constant(f32::INFINITY)),
                    ..Default::default()
                },
                ProposalSize::new(Some(10.0), Some(f32::INFINITY)),
                Rect::from_size(Size::new(10.0, 100.0)),
                Rect::new(Point::new(-5.0, 45.0), Size::new(20.0, 10.0)),
            ),
        ] {
            let child = MockSubView {
                size: Size::new(20.0, 10.0),
            };
            assert_eq!(
                layout.size_that_fits(proposal, &[&child]),
                Size::new(20.0, 10.0),
                "an infinite probe meets a finite child: the frame's answer stays finite"
            );

            // Under later finite bounds ordinary filling placement is
            // unchanged: the child centers in the offered extent.
            let placements = layout.place(bounds, proposal, &[&child]);
            assert_eq!(placements[0].frame, expected_frame);
        }
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

        let proposal = ProposalSize::new(Some(320.0), Some(180.0));
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(320.0, 180.0));
        let placements = layout.place(bounds, proposal, &[&child]);

        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(320.0), Some(180.0)),
            "the child should be re-proposed what it was measured with"
        );
        assert_extent(placements[0].frame.width(), 320.0, "the child's width");
        assert_extent(placements[0].frame.height(), 180.0, "the child's height");
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

        let placements = layout.place(
            Rect::new(Point::new(0.0, 0.0), size),
            ProposalSize::UNSPECIFIED,
            &[&child],
        );
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(100.0), Some(50.0)),
            "the child should be proposed the ideal the frame resolved to"
        );
        assert_extent(placements[0].frame.width(), 100.0, "the child's width");
        assert_extent(placements[0].frame.height(), 50.0, "the child's height");
    }

    #[test]
    fn placement_holds_the_frames_own_minimum_and_maximum_bounded_proposal() {
        // `.frame(minWidth: 120, maxHeight: 40)` handed an 80x300 slot: the
        // frame's own constraints still bind on the proposal it passes down,
        // so the child hears 120 wide and 40 tall rather than the raw offer.
        let layout = FrameLayout {
            min_width: Some(Computed::constant(120.0)),
            max_height: Some(Computed::constant(40.0)),
            ..Default::default()
        };

        let child = RecordingSubView::new(FillingSubView {
            intrinsic: Size::new(30.0, 20.0),
        });

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(80.0, 300.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        layout.place(bounds, proposal, &[&child]);

        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(120.0), Some(40.0)),
            "the child's proposal should be the offer clamped by the frame"
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

    /// A child whose width follows the height it is proposed: it answers
    /// `min(proposedHeight, 80)` wide and 20 tall, reading 20 for an
    /// unspecified height. Its width therefore changes with the height axis,
    /// which is what a stale placement proposal corrupts.
    struct HeightCoupledSubView;

    impl SubView for HeightCoupledSubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            let offered_height = proposal.height.unwrap_or(20.0);
            ViewDimensions::new(Size::new(offered_height.min(80.0), 20.0))
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    /// The transposed counterpart of [`HeightCoupledSubView`]: its height
    /// follows the width it is proposed.
    struct WidthCoupledSubView;

    impl SubView for WidthCoupledSubView {
        fn measure(&self, proposal: ProposalSize) -> ViewDimensions {
            let offered_width = proposal.width.unwrap_or(20.0);
            ViewDimensions::new(Size::new(20.0, offered_width.min(80.0)))
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn frame_unconstrained_axis_uses_resolved_bounds() {
        // `.frame(maxWidth: .infinity)` around a child whose width follows
        // the height it is proposed, offered 100x80. The frame fills the
        // width and keeps the child's 20pt height; placement negotiates on
        // both axes, so the child is re-proposed the resolved 100x20 region —
        // never the stale 100x80 offer.
        let layout = FrameLayout {
            max_width: Some(Computed::constant(f32::INFINITY)),
            ..Default::default()
        };
        let child = RecordingSubView::new(HeightCoupledSubView);

        let proposal = ProposalSize::new(Some(100.0), Some(80.0));
        let size = layout.size_that_fits(proposal, &[&child]);
        assert_eq!(size, Size::new(100.0, 20.0));

        let placements = layout.place(Rect::new(Point::new(0.0, 0.0), size), proposal, &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(100.0), Some(20.0)),
            "the unconstrained height axis re-proposes the resolved bounds"
        );
        assert_eq!(
            placements[0].frame,
            Rect::new(Point::new(40.0, 0.0), Size::new(20.0, 20.0))
        );

        // The transpose: `.frame(maxHeight: .infinity)` with the axes
        // swapped, offered 80x100.
        let layout = FrameLayout {
            max_height: Some(Computed::constant(f32::INFINITY)),
            ..Default::default()
        };
        let child = RecordingSubView::new(WidthCoupledSubView);

        let proposal = ProposalSize::new(Some(80.0), Some(100.0));
        let size = layout.size_that_fits(proposal, &[&child]);
        assert_eq!(size, Size::new(20.0, 100.0));

        let placements = layout.place(Rect::new(Point::new(0.0, 0.0), size), proposal, &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(20.0), Some(100.0)),
            "the unconstrained width axis re-proposes the resolved bounds"
        );
        assert_eq!(
            placements[0].frame,
            Rect::new(Point::new(0.0, 40.0), Size::new(20.0, 20.0))
        );

        // An axis that carries only an ideal still re-proposes the resolved
        // bounds, not the unspecified offer it answered through the ideal.
        let layout = FrameLayout {
            max_width: Some(Computed::constant(f32::INFINITY)),
            ideal_height: Some(Computed::constant(60.0)),
            ..Default::default()
        };
        let child = RecordingSubView::new(HeightCoupledSubView);

        let proposal = ProposalSize::new(Some(100.0), None);
        let size = layout.size_that_fits(proposal, &[&child]);
        assert_eq!(size, Size::new(100.0, 20.0));

        let placements = layout.place(Rect::new(Point::new(0.0, 0.0), size), proposal, &[&child]);
        assert_eq!(
            child.proposal(),
            ProposalSize::new(Some(100.0), Some(20.0)),
            "an ideal-only axis re-proposes the resolved bounds"
        );
        assert_eq!(
            placements[0].frame,
            Rect::new(Point::new(40.0, 0.0), Size::new(20.0, 20.0))
        );
    }
}
