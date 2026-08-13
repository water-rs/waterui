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
        // A Frame proposes a modified size to its single child.
        // It uses its own ideal dimensions if they exist, otherwise parent's proposal,
        // then clamps that proposal by the frame's min/max constraints.
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

        // Resolve the frame size on each axis.
        //
        // If parent proposes a concrete size, we respect it but clamp through frame constraints.
        // If parent leaves axis unspecified, use frame ideal or measured child size, then clamp.
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
        let proposed_width = resolved.ideal_width.unwrap_or_else(|| bounds.width());
        let proposed_height = resolved.ideal_height.unwrap_or_else(|| bounds.height());

        let child_proposal = ProposalSize {
            width: Some(clamp_frame_axis(
                proposed_width.min(bounds.width()),
                resolved.min_width,
                resolved.max_width,
            )),
            height: Some(clamp_frame_axis(
                proposed_height.min(bounds.height()),
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

#[inline]
fn frame_child_proposal_axis(
    parent_proposal: Option<f32>,
    min: Option<f32>,
    ideal: Option<f32>,
    max: Option<f32>,
) -> Option<f32> {
    ideal
        .or(parent_proposal)
        .map(|value| clamp_frame_axis(value, min, max))
}

/// Resolves the frame's own extent on one axis.
///
/// A frame grows into the space offered *up to* the maximum it was given, which
/// is how `.frame(maxWidth: .infinity)` fills and `.frame(maxWidth: 100)` fills
/// to a hundred points. Without a maximum there is nothing to grow into, so the
/// frame stays the size of its content — `.frame(minHeight: 44)` on a label
/// leaves a 44pt-tall label, rather than a label stretched over whatever height
/// the parent happened to propose.
#[inline]
fn frame_resolved_axis(
    parent_proposal: Option<f32>,
    child_size: f32,
    min: Option<f32>,
    ideal: Option<f32>,
    max: Option<f32>,
) -> f32 {
    let content = ideal.unwrap_or(child_size);
    match (parent_proposal, max) {
        (Some(proposed), Some(_)) => clamp_frame_axis(proposed, min, max),
        _ => clamp_frame_axis(content, min, max),
    }
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

    #[test]
    fn test_frame_with_ideal_size() {
        let layout = FrameLayout {
            ideal_width: Some(Computed::constant(100.0)),
            ideal_height: Some(Computed::constant(50.0)),
            ..Default::default()
        };

        let mut child = MockSubView {
            size: Size::new(30.0, 20.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Frame uses ideal dimensions
        assert!((size.width - 100.0).abs() < f32::EPSILON);
        assert!((size.height - 50.0).abs() < f32::EPSILON);
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

        let child = MockSubView {
            size: Size::new(30.0, 20.0),
        };

        // Nothing proposed: the frame asks for its ideal rather than the child's.
        let unconstrained = layout.size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
        assert!(
            (unconstrained.width - 120.0).abs() < f32::EPSILON,
            "expected the ideal width, got {}",
            unconstrained.width
        );

        // An ideal alone sets no maximum, so a parent that does propose a size
        // does not get to stretch the frame past its content.
        let proposed = layout.size_that_fits(ProposalSize::new(Some(300.0), None), &[&child]);
        assert!(
            (proposed.width - 120.0).abs() < f32::EPSILON,
            "an ideal width is not a licence to fill, got {}",
            proposed.width
        );
    }
}
