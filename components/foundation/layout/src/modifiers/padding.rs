//! Padding layouts that inset a child by fixed edge distances.

use alloc::{vec, vec::Vec};
use nami::{Computed, Signal, signal::IntoComputed, watcher::BoxWatcherGuard};
use waterui_core::{AnyView, View, layout::LayoutInvalidationCallback};

use crate::{
    HorizontalAlignment, Layout, PlacedSubview, Point, ProposalSize, Rect, Size, StretchAxis,
    SubView, SubviewPlacement, VerticalAlignment, container::FixedContainer,
};

/// Layout that insets its single child by the configured edge values.
///
/// The insets are reactive, so a change to them moves the padded content
/// without the subtree being rebuilt.
#[derive(Debug, Clone)]
pub struct PaddingLayout {
    edges: Computed<EdgeInsets>,
}

impl Layout for PaddingLayout {
    /// Padding is transparent to its content: it insets the child within
    /// whatever bounds it is given, so the child's axis is the answer.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        children.first().copied().unwrap_or_default()
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let edges = self.edges.get();
        // The horizontal and vertical space consumed by padding.
        let horizontal_padding = edges.leading + edges.trailing;
        let vertical_padding = edges.top + edges.bottom;

        // Reduce the proposed size for the child by the padding amount.
        let child_proposal = ProposalSize {
            width: proposal.width.map(|w| (w - horizontal_padding).max(0.0)),
            height: proposal.height.map(|h| (h - vertical_padding).max(0.0)),
        };

        // Measure the child
        let child_size = children
            .first()
            .map_or(Size::zero(), |c| c.measure(child_proposal).size);

        // A greedy child reports an infinite extent, which cannot be added to the
        // insets, so resolve it against the offer instead. With nothing offered
        // there is nothing to fill, and the inset alone is the answer — never a
        // negative extent, which is what subtracting the insets from a missing
        // proposal used to produce.
        let child_width = if child_size.width.is_infinite() {
            (proposal.width.unwrap_or(0.0) - horizontal_padding).max(0.0)
        } else {
            child_size.width
        };

        let child_height = if child_size.height.is_infinite() {
            (proposal.height.unwrap_or(0.0) - vertical_padding).max(0.0)
        } else {
            child_size.height
        };

        // The final size is the child's size plus the padding.
        Size::new(
            child_width + horizontal_padding,
            child_height + vertical_padding,
        )
    }

    fn place(
        &self,
        bounds: Rect,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        if children.is_empty() {
            return vec![];
        }

        let edges = self.edges.get();
        // Create the child's frame by insetting the parent's bound by the padding amount.
        let child_origin = Point::new(bounds.x() + edges.leading, bounds.y() + edges.top);

        let horizontal_padding = edges.leading + edges.trailing;
        let vertical_padding = edges.top + edges.bottom;

        let child_size = Size::new(
            (bounds.width() - horizontal_padding).max(0.0),
            (bounds.height() - vertical_padding).max(0.0),
        );

        // The child is measured and placed under the same inset-adjusted
        // proposal `size_that_fits` offers it.
        let child_proposal = ProposalSize::new(
            proposal.width.map(|w| (w - horizontal_padding).max(0.0)),
            proposal.height.map(|h| (h - vertical_padding).max(0.0)),
        );

        vec![SubviewPlacement::new(
            Rect::new(child_origin, child_size),
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

    fn watch_invalidation(&self, invalidate: LayoutInvalidationCallback) -> Vec<BoxWatcherGuard> {
        vec![self.edges.watch(move |_| invalidate())]
    }
}

/// Insets applied to the four edges of a rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct EdgeInsets {
    top: f32,
    bottom: f32,
    leading: f32,
    trailing: f32,
}

nami::impl_constant!(EdgeInsets);

/// Equal insets on every edge from one number — `.padding_with(16.0)` —
/// for every numeric type `f64` converts from, which is the set the former
/// `From<T: Into<f64>>` blanket covered. The blanket had to go so the tuple
/// and array conversions below could exist beside it (coherence forbids a
/// concrete `From<(f32, f32)>` next to a blanket over `Into<f64>`).
macro_rules! from_scalar {
    (lossless: $($ty:ty),+ $(,)?) => {
        $(
            impl From<$ty> for EdgeInsets {
                fn from(value: $ty) -> Self {
                    Self::all(f32::from(value))
                }
            }
        )+
    };
    (lossy: $($ty:ty),+ $(,)?) => {
        $(
            #[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
            impl From<$ty> for EdgeInsets {
                fn from(value: $ty) -> Self {
                    Self::all(value as f32)
                }
            }
        )+
    };
}

from_scalar!(lossless: f32, i8, i16, u8, u16);
from_scalar!(lossy: f64, i32, u32);

/// `(vertical, horizontal)` — the [`EdgeInsets::symmetric`] order:
/// `.padding_with((8.0, 16.0))`. The `f64` form is what an unsuffixed
/// literal pair infers to as a constant signal.
macro_rules! from_pair {
    ($($ty:ty),+ $(,)?) => {
        $(
            #[allow(clippy::cast_possible_truncation)]
            impl From<($ty, $ty)> for EdgeInsets {
                fn from((vertical, horizontal): ($ty, $ty)) -> Self {
                    Self::symmetric(vertical as f32, horizontal as f32)
                }
            }
        )+
    };
}

from_pair!(f32, f64);

/// `[top, bottom, leading, trailing]` — the [`EdgeInsets::new`] order:
/// `.padding_with([4.0, 12.0, 16.0, 16.0])`.
macro_rules! from_quad {
    ($($ty:ty),+ $(,)?) => {
        $(
            #[allow(clippy::cast_possible_truncation)]
            impl From<[$ty; 4]> for EdgeInsets {
                fn from([top, bottom, leading, trailing]: [$ty; 4]) -> Self {
                    Self::new(top as f32, bottom as f32, leading as f32, trailing as f32)
                }
            }
        )+
    };
}

from_quad!(f32, f64);

impl core::ops::Add for EdgeInsets {
    type Output = Self;

    /// Stacks two sets of insets, edge by edge.
    fn add(self, rhs: Self) -> Self {
        Self {
            top: self.top + rhs.top,
            bottom: self.bottom + rhs.bottom,
            leading: self.leading + rhs.leading,
            trailing: self.trailing + rhs.trailing,
        }
    }
}

impl Default for EdgeInsets {
    fn default() -> Self {
        Self::all(0.0)
    }
}

impl EdgeInsets {
    /// Creates an [`EdgeInsets`] value with explicit edges.
    #[must_use]
    pub const fn new(top: f32, bottom: f32, leading: f32, trailing: f32) -> Self {
        Self {
            top,
            bottom,
            leading,
            trailing,
        }
    }

    /// Returns equal insets on every edge.
    #[must_use]
    pub const fn all(value: f32) -> Self {
        Self {
            top: value,
            bottom: value,
            leading: value,
            trailing: value,
        }
    }

    /// Returns symmetric vertical and horizontal insets.
    #[must_use]
    pub const fn symmetric(vertical: f32, horizontal: f32) -> Self {
        Self {
            top: vertical,
            bottom: vertical,
            leading: horizontal,
            trailing: horizontal,
        }
    }

    /// Returns the top inset.
    #[must_use]
    pub const fn top(&self) -> f32 {
        self.top
    }

    /// Returns the bottom inset.
    #[must_use]
    pub const fn bottom(&self) -> f32 {
        self.bottom
    }

    /// Returns the leading (left in LTR) inset.
    #[must_use]
    pub const fn leading(&self) -> f32 {
        self.leading
    }

    /// Returns the trailing (right in LTR) inset.
    #[must_use]
    pub const fn trailing(&self) -> f32 {
        self.trailing
    }
}

/// View wrapper that applies [`PaddingLayout`] to a single child.
#[derive(Debug)]
pub struct Padding {
    layout: PaddingLayout,
    content: AnyView,
}

impl Padding {
    /// Wraps a view with custom `edges`.
    pub fn new(edges: impl IntoComputed<EdgeInsets>, content: impl View + 'static) -> Self {
        Self {
            layout: PaddingLayout {
                edges: edges.into_computed(),
            },
            content: AnyView::new(content),
        }
    }

    /// Consumes the padding and returns the edge insets and content.
    pub fn into_inner(self) -> (Computed<EdgeInsets>, AnyView) {
        (self.layout.edges, self.content)
    }
}

impl View for Padding {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
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
    use crate::ViewDimensions;
    use crate::measure_layout;
    use alloc::rc::Rc;
    use core::cell::Cell;
    use nami::binding;

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

    struct GuidedSubview {
        size: Size,
        horizontal_guide: f32,
        vertical_guide: f32,
    }

    impl SubView for GuidedSubview {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.size)
                .with_horizontal(HorizontalAlignment::Leading, self.horizontal_guide)
                .with_vertical(VerticalAlignment::Top, self.vertical_guide)
        }

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }

        fn priority(&self) -> i32 {
            0
        }
    }

    #[test]
    fn test_padding_size() {
        let layout = PaddingLayout {
            edges: EdgeInsets::all(10.0).into_computed(),
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Size = child size + padding on all sides
        assert!((size.width - 70.0).abs() < f32::EPSILON); // 50 + 10 + 10
        assert!((size.height - 50.0).abs() < f32::EPSILON); // 30 + 10 + 10
    }

    #[test]
    fn test_padding_placement_bounded_proposal() {
        let layout = PaddingLayout {
            edges: EdgeInsets::new(10.0, 20.0, 15.0, 25.0).into_computed(),
        };

        let mut child = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let children: Vec<&dyn SubView> = vec![&mut child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout.place(bounds, proposal, &children);

        // Child origin is offset by leading and top
        assert!((placements[0].frame.x() - 15.0).abs() < f32::EPSILON);
        assert!((placements[0].frame.y() - 10.0).abs() < f32::EPSILON);

        // Child size is bounds minus padding
        assert!((placements[0].frame.width() - 60.0).abs() < f32::EPSILON); // 100 - 15 - 25
        assert!((placements[0].frame.height() - 70.0).abs() < f32::EPSILON); // 100 - 10 - 20
    }

    #[test]
    fn reactive_insets_invalidate_and_change_measurement() {
        let inset = binding(EdgeInsets::all(10.0));
        let layout = PaddingLayout {
            edges: inset.clone().into_computed(),
        };

        let invalidations = Rc::new(Cell::new(0));
        let counted = Rc::clone(&invalidations);
        let _guards = layout.watch_invalidation(Rc::new(move || {
            counted.set(counted.get() + 1);
        }));

        let child = MockSubView {
            size: Size::new(50.0, 30.0),
        };
        let children: Vec<&dyn SubView> = vec![&child];
        assert_eq!(
            layout
                .size_that_fits(ProposalSize::UNSPECIFIED, &children)
                .width,
            70.0
        );

        inset.set(EdgeInsets::all(20.0));

        assert_eq!(invalidations.get(), 1);
        assert_eq!(
            layout
                .size_that_fits(ProposalSize::UNSPECIFIED, &children)
                .width,
            90.0
        );
    }

    #[test]
    fn test_padding_offsets_explicit_guides() {
        let layout = PaddingLayout {
            edges: EdgeInsets::new(10.0, 0.0, 15.0, 0.0).into_computed(),
        };
        let child = GuidedSubview {
            size: Size::new(50.0, 30.0),
            horizontal_guide: 8.0,
            vertical_guide: 6.0,
        };
        let children: Vec<&dyn SubView> = vec![&child];

        let dimensions = measure_layout(&layout, ProposalSize::UNSPECIFIED, &children);

        assert_eq!(
            dimensions.explicit_horizontal(HorizontalAlignment::Leading),
            Some(23.0)
        );
        assert_eq!(
            dimensions.explicit_vertical(VerticalAlignment::Top),
            Some(16.0)
        );
    }
}
