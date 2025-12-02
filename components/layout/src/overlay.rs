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
    Layout, Point, ProposalSize, Rect, Size, SubView,
    container::FixedContainer, stack::Alignment,
};

/// Cached measurement for a child during layout
struct ChildMeasurement {
    size: Size,
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

    fn aligned_origin(&self, bounds: &Rect, size: &Size) -> Point {
        match self.alignment {
            Alignment::TopLeading => Point::new(bounds.x(), bounds.y()),
            Alignment::Top => {
                Point::new(bounds.x() + (bounds.width() - size.width) / 2.0, bounds.y())
            }
            Alignment::TopTrailing => Point::new(bounds.max_x() - size.width, bounds.y()),
            Alignment::Leading => Point::new(
                bounds.x(),
                bounds.y() + (bounds.height() - size.height) / 2.0,
            ),
            Alignment::Center => Point::new(
                bounds.x() + (bounds.width() - size.width) / 2.0,
                bounds.y() + (bounds.height() - size.height) / 2.0,
            ),
            Alignment::Trailing => Point::new(
                bounds.max_x() - size.width,
                bounds.y() + (bounds.height() - size.height) / 2.0,
            ),
            Alignment::BottomLeading => Point::new(bounds.x(), bounds.max_y() - size.height),
            Alignment::Bottom => Point::new(
                bounds.x() + (bounds.width() - size.width) / 2.0,
                bounds.max_y() - size.height,
            ),
            Alignment::BottomTrailing => {
                Point::new(bounds.max_x() - size.width, bounds.max_y() - size.height)
            }
        }
    }
}

impl Layout for OverlayLayout {
    fn size_that_fits(
        &self,
        proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Size {
        // Overlay size is driven entirely by the base child (index 0). If the base
        // provides no intrinsic size, fall back to the parent's constraints.
        let base_size = children
            .first()
            .map(|c| c.size_that_fits(proposal))
            .unwrap_or(Size::zero());

        let base_width = if base_size.width.is_finite() && base_size.width > 0.0 {
            base_size.width
        } else {
            proposal.width.unwrap_or(0.0)
        };

        let base_height = if base_size.height.is_finite() && base_size.height > 0.0 {
            base_size.height
        } else {
            proposal.height.unwrap_or(0.0)
        };

        let width = proposal.width.unwrap_or(base_width);
        let height = proposal.height.unwrap_or(base_height);

        Size::new(width.max(0.0), height.max(0.0))
    }

    fn place(
        &self,
        bounds: Rect,
        children: &[&dyn SubView],
    ) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        // Measure all children
        let child_proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));

        let measurements: Vec<ChildMeasurement> = children
            .iter()
            .map(|child| ChildMeasurement {
                size: child.size_that_fits(child_proposal),
            })
            .collect();

        let mut placements = Vec::with_capacity(children.len());

        // Base child always fills the container's bounds
        if let Some(base) = measurements.first() {
            let base_width = if base.size.width.is_infinite() {
                bounds.width()
            } else {
                base.size.width
            };
            let base_height = if base.size.height.is_infinite() {
                bounds.height()
            } else {
                base.size.height
            };
            placements.push(Rect::new(bounds.origin(), Size::new(base_width, base_height)));
        }

        // Overlay children are aligned within the bounds
        for measurement in measurements.iter().skip(1) {
            let width = if measurement.size.width.is_infinite() {
                bounds.width()
            } else {
                measurement.size.width.min(bounds.width()).max(0.0)
            };
            let height = if measurement.size.height.is_infinite() {
                bounds.height()
            } else {
                measurement.size.height.min(bounds.height()).max(0.0)
            };
            let size = Size::new(width, height);
            let origin = self.aligned_origin(&bounds, &size);
            placements.push(Rect::new(origin, size));
        }

        placements
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
        FixedContainer::new(self.layout, (self.base, self.layer))
    }
}

/// Convenience constructor for creating an [`Overlay`] with the default alignment.
#[must_use]
pub const fn overlay<Base, Layer>(base: Base, layer: Layer) -> Overlay<Base, Layer> {
    Overlay::new(base, layer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StretchAxis;

    struct MockSubView {
        size: Size,
    }

    impl SubView for MockSubView {
        fn size_that_fits(&self, _proposal: ProposalSize) -> Size {
            self.size
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
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
}
