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
    ChildMetadata, Layout, Point, ProposalSize, Rect, Size, container::FixedContainer,
    stack::Alignment,
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
    fn propose(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Vec<ProposalSize> {
        // Forward the parent's proposal verbatim. The overlay only cares about two
        // children, but extra entries are handled gracefully.
        vec![parent; children.len()]
    }

    fn size(&mut self, parent: ProposalSize, children: &[ChildMetadata]) -> Size {
        // Overlay size is driven entirely by the base child (index 0). If the base
        // provides no intrinsic size, fall back to the parent's constraints.
        let base_width = children
            .first()
            .and_then(ChildMetadata::proposal_width)
            .or(parent.width)
            .unwrap_or(0.0);
        let base_height = children
            .first()
            .and_then(ChildMetadata::proposal_height)
            .or(parent.height)
            .unwrap_or(0.0);

        let width = parent.width.map_or(base_width, |w| w);
        let height = parent.height.map_or(base_height, |h| h);

        Size::new(width.max(0.0), height.max(0.0))
    }

    fn place(
        &mut self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[ChildMetadata],
    ) -> Vec<Rect> {
        if children.is_empty() {
            return Vec::new();
        }

        let mut placements = Vec::with_capacity(children.len());

        // Base child always fills the container's bounds.
        placements.push(bounds.clone());

        for child in children.iter().skip(1) {
            let width = child
                .proposal_width()
                .unwrap_or_else(|| bounds.width())
                .min(bounds.width())
                .max(0.0);
            let height = child
                .proposal_height()
                .unwrap_or_else(|| bounds.height())
                .min(bounds.height())
                .max(0.0);
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
