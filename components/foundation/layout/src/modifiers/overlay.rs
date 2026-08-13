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
    Layout, PlacedSubview, Point, ProposalSize, Rect, Size, StretchAxis, SubView, ViewDimensions,
    container::FixedContainer,
    stack::{Alignment, HorizontalAlignment, VerticalAlignment},
};

/// Cached measurement for a child during layout
struct ChildMeasurement {
    dimensions: ViewDimensions,
}

/// Layout used by [`Overlay`] to keep the base child's size authoritative while
/// still allowing aligned overlay content.
#[derive(Debug, Clone, Default)]
pub struct OverlayLayout {
    alignment: Alignment,
}

impl OverlayLayout {
    /// Sets the [`Alignment`] used to position overlay layers relative to the base.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
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

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Measure all children
        let child_proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));

        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                dimensions: child.measure(child_proposal),
            })
            .collect();

        let mut placements = Vec::with_capacity(children.len());

        if let Some(base) = measurements.first() {
            let base_width = overlay_placement_axis(base.dimensions.size.width, bounds.width());
            let base_height = overlay_placement_axis(base.dimensions.size.height, bounds.height());
            placements.push(Rect::new(
                bounds.origin(),
                Size::new(base_width, base_height),
            ));
        }

        // Overlay children are aligned within the bounds
        for measurement in measurements.iter().skip(1) {
            let child_dimensions = &measurement.dimensions;
            let width = if child_dimensions.size.width.is_infinite() {
                bounds.width()
            } else {
                child_dimensions.size.width.min(bounds.width()).max(0.0)
            };
            let height = if child_dimensions.size.height.is_infinite() {
                bounds.height()
            } else {
                child_dimensions.size.height.min(bounds.height()).max(0.0)
            };
            let size = Size::new(width, height);
            let mut adjusted_dimensions = child_dimensions.clone();
            adjusted_dimensions.size = size;
            let horizontal = self.alignment.horizontal();
            let vertical = self.alignment.vertical();
            let target_x = if horizontal == HorizontalAlignment::Leading {
                0.0
            } else if horizontal == HorizontalAlignment::Trailing {
                bounds.width()
            } else if horizontal == HorizontalAlignment::Center {
                bounds.width() * 0.5
            } else {
                adjusted_dimensions
                    .horizontal(horizontal)
                    .clamp(0.0, size.width)
            };
            let target_y = if vertical == VerticalAlignment::Top {
                0.0
            } else if vertical == VerticalAlignment::Bottom {
                bounds.height()
            } else if vertical == VerticalAlignment::Center {
                bounds.height() * 0.5
            } else {
                adjusted_dimensions
                    .vertical(vertical)
                    .clamp(0.0, size.height)
            };
            let origin = Point::new(
                bounds.x() + target_x
                    - adjusted_dimensions
                        .horizontal(horizontal)
                        .clamp(0.0, size.width),
                bounds.y() + target_y
                    - adjusted_dimensions
                        .vertical(vertical)
                        .clamp(0.0, size.height),
            );
            placements.push(Rect::new(origin, size));
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

const fn overlay_placement_axis(measured: f32, bounds_axis: f32) -> f32 {
    if measured.is_infinite() {
        bounds_axis.max(0.0)
    } else {
        measured.min(bounds_axis).max(0.0)
    }
}

/// A view that layers `overlay` content on top of a `base` view without
/// allowing the overlay to influence layout sizing.
pub struct Overlay<Base, Layer> {
    layout: OverlayLayout,
    base: Base,
    layer: Layer,
}

impl<Base, Layer> Overlay<Base, Layer> {
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
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.layout.alignment = alignment;
        self
    }
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
}

/// Convenience constructor for creating an [`Overlay`] with the default alignment.
#[must_use]
pub const fn overlay<Base, Layer>(base: Base, layer: Layer) -> Overlay<Base, Layer> {
    Overlay::new(base, layer)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;
    use crate::StretchAxis;
    use crate::container::FixedContainer;
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
    fn test_overlay_placement_center() {
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
        let rects = layout.place(bounds, &children);

        // Base fills bounds
        assert_eq!(rects[0].width(), 100.0);
        assert_eq!(rects[0].height(), 100.0);

        // Overlay child centered
        assert_eq!(rects[1].x(), 40.0); // (100 - 20) / 2
        assert_eq!(rects[1].y(), 40.0); // (100 - 20) / 2
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
