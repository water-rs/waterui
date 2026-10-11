//! Overlay helpers for layering content on top of a base view.
//!
//! `overlay` mirrors the intent of a two-child `ZStack`, but the container's
//! dimensions are locked to the first (base) child. This makes it ideal for
//! badges, highlights, and decorators that should not influence the parent
//! layout's sizing decisions.

use core::fmt;

use alloc::{vec, vec::Vec};
use waterui_core::View;

use crate::{
    Layout, PlacedSubview, Point, ProposalSize, Rect, Size, StretchAxis, SubView, SubviewPlacement,
    container::FixedContainer,
    stack::{
        Alignment, HorizontalAlignment, VerticalAlignment,
        distribute::{LineAnchor, container_line},
    },
};

/// Layout used by [`Overlay`] to keep the base child's size authoritative while
/// still allowing aligned overlay content.
#[derive(Debug, Clone, Default)]
pub struct OverlayLayout {
    alignment: Alignment,
}

impl OverlayLayout {
    /// Sets the [`Alignment`] used to position overlay layers relative to the base.
    #[must_use]
    pub fn alignment(mut self, alignment: impl Into<Alignment>) -> Self {
        self.alignment = alignment.into();
        self
    }

    /// Returns the current alignment.
    #[must_use]
    pub const fn alignment_ref(&self) -> Alignment {
        self.alignment
    }
}

impl Layout for OverlayLayout {
    /// An overlay is sized by its base, `children[0]`; the layer on top never
    /// changes what the pair claims.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        children.first().copied().unwrap_or_default()
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let base_size = children
            .first()
            .map_or(Size::zero(), |c| c.measure(proposal).size);

        Size::new(
            overlay_axis_size(base_size.width, proposal.width),
            overlay_axis_size(base_size.height, proposal.height),
        )
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

        // The wrapper is transparent: the base takes the whole bounds and is
        // proposed them, and each decoration is proposed the same extent. A
        // decoration keeps its own answer, overflowing the base when it is
        // larger, and sits with its guide on the base's alignment line.
        let placement = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));

        let mut placements = Vec::with_capacity(children.len());
        placements.push(SubviewPlacement::new(bounds, placement));

        let horizontal = self.alignment.horizontal();
        let vertical = self.alignment.vertical();
        for child in children.iter().skip(1) {
            let mut dimensions = child.measure(placement);
            let width = if dimensions.size.width.is_infinite() {
                bounds.width()
            } else {
                dimensions.size.width.max(0.0)
            };
            let height = if dimensions.size.height.is_infinite() {
                bounds.height()
            } else {
                dimensions.size.height.max(0.0)
            };
            dimensions.size = Size::new(width, height);
            let guide_x = dimensions.horizontal(horizontal);
            let guide_y = dimensions.vertical(vertical);
            let line_x = container_line(
                bounds.width(),
                LineAnchor::horizontal(horizontal),
                guide_x,
                width - guide_x,
            );
            let line_y = container_line(
                bounds.height(),
                LineAnchor::vertical(vertical),
                guide_y,
                height - guide_y,
            );
            let origin = Point::new(bounds.x() + line_x - guide_x, bounds.y() + line_y - guide_y);
            placements.push(SubviewPlacement::new(
                Rect::new(origin, dimensions.size),
                placement,
            ));
        }

        placements
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
}

fn overlay_axis_size(measured: f32, proposal: Option<f32>) -> f32 {
    if measured.is_finite() {
        measured.max(0.0)
    } else {
        proposal.unwrap_or(0.0).max(0.0)
    }
}

/// A view that layers `overlay` content on top of a `base` view without
/// allowing the overlay to influence layout sizing.
pub struct Overlay<Base, Layer> {
    layout: OverlayLayout,
    base: Base,
    layer: Layer,
}

impl<Base: View, Layer: View> Overlay<Base, Layer> {
    /// Creates a new overlay using the provided base view and overlay layer.
    #[must_use]
    pub const fn new(base: Base, layer: Layer) -> Self {
        Self {
            layout: OverlayLayout {
                alignment: Alignment::Center,
            },
            base,
            layer,
        }
    }

    /// Sets how the overlay layer should be aligned inside the base bounds.
    #[must_use]
    pub fn alignment(mut self, alignment: impl Into<Alignment>) -> Self {
        self.layout.alignment = alignment.into();
        self
    }

    crate::alignment::two_dimensional_alignment_methods!();
}

impl<Base, Layer> fmt::Debug for Overlay<Base, Layer> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Overlay")
            .field("layout", &self.layout)
            .finish_non_exhaustive()
    }
}

impl<Base, Layer> View for Overlay<Base, Layer>
where
    Base: View + 'static,
    Layer: View + 'static,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let Self {
            layout,
            base,
            layer,
        } = self;
        FixedContainer::new(layout, (base, layer))
    }

    /// Resolves to `FixedContainer` over the same layout and children in
    /// `[base, layer]` order; reports what that container would.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout
            .stretch_axis(&[self.base.stretch_axis(), self.layer.stretch_axis()])
    }
}

/// Convenience constructor for creating an [`Overlay`] with the default alignment.
#[must_use]
pub const fn overlay<Base: View, Layer: View>(base: Base, layer: Layer) -> Overlay<Base, Layer> {
    Overlay::new(base, layer)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::container::FixedContainer;
    use crate::{StretchAxis, ViewDimensions};
    use waterui_core::{AnyView, Environment, View};

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

    struct StretchingBase;

    impl View for StretchingBase {
        fn body(self, _env: &Environment) -> impl View {}

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::Horizontal
        }
    }

    #[test]
    fn test_overlay_size_from_base() {
        let layout = OverlayLayout::default();

        let mut base = MockSubView {
            size: Size::new(100.0, 50.0),
        };
        let mut overlay_child = MockSubView {
            size: Size::new(20.0, 20.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut base, &mut overlay_child];

        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Size comes from base child
        assert_eq!(size.width, 100.0);
        assert_eq!(size.height, 50.0);
    }

    #[test]
    fn test_overlay_placement_center_bounded_proposal() {
        let layout = OverlayLayout {
            alignment: Alignment::Center,
        };

        let mut base = MockSubView {
            size: Size::new(100.0, 100.0),
        };
        let mut overlay_child = MockSubView {
            size: Size::new(20.0, 20.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut base, &mut overlay_child];

        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout.place(bounds, proposal, &children);

        // Base fills bounds
        assert_eq!(placements[0].frame.width(), 100.0);
        assert_eq!(placements[0].frame.height(), 100.0);

        // Overlay child centered
        assert_eq!(placements[1].frame.x(), 40.0); // (100 - 20) / 2
        assert_eq!(placements[1].frame.y(), 40.0); // (100 - 20) / 2
    }

    #[test]
    fn test_overlay_preserves_intrinsic_base_size_under_parent_proposal() {
        let layout = OverlayLayout::default();

        let base = MockSubView {
            size: Size::new(40.0, 30.0),
        };
        let overlay_child = MockSubView {
            size: Size::new(10.0, 10.0),
        };

        let children: Vec<&dyn SubView> = vec![&base, &overlay_child];
        let size = layout.size_that_fits(ProposalSize::new(Some(200.0), Some(100.0)), &children);

        assert_eq!(size.width, 40.0);
        assert_eq!(size.height, 30.0);
    }

    #[test]
    fn test_overlay_body_preserves_base_stretch_axis() {
        let env = Environment::default();
        let body = overlay(StretchingBase, ()).body(&env);
        let container = AnyView::new(body)
            .downcast::<FixedContainer>()
            .expect("overlay body should be a FixedContainer");
        let (layout, children) = container.as_parts();

        assert_eq!(children.len(), 2);
        let child_axes: Vec<StretchAxis> = children.iter().map(View::stretch_axis).collect();
        assert_eq!(layout.stretch_axis(&child_axes), StretchAxis::Horizontal);
    }
}
