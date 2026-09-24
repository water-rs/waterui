//! Alignment-guide wrapper views.
//!
//! These wrappers are layout-transparent: they preserve the wrapped view's size,
//! placement, and stretch behavior, while overriding the guide value exposed to
//! parent layouts.

use core::fmt;

use alloc::{vec, vec::Vec};
use waterui_core::View;

use crate::{
    HorizontalAlignment, Layout, PlacedSubview, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement, VerticalAlignment, ViewDimensions, container::FixedContainer,
};

#[derive(Clone)]
struct HorizontalAlignmentGuideLayout<F> {
    alignment: HorizontalAlignment,
    compute: F,
}

impl<F> fmt::Debug for HorizontalAlignmentGuideLayout<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HorizontalAlignmentGuideLayout")
            .field("alignment", &self.alignment)
            .finish_non_exhaustive()
    }
}

impl<F> Layout for HorizontalAlignmentGuideLayout<F>
where
    F: Fn(&ViewDimensions) -> f32 + 'static,
{
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        assert!(
            children.len() == 1,
            "HorizontalAlignmentGuideLayout expects exactly one child"
        );
        children[0].measure(proposal).size
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        assert!(
            children.len() == 1,
            "HorizontalAlignmentGuideLayout expects exactly one child"
        );
        // Placement negotiates on both axes against the bounds: the child is
        // proposed the region it is actually placed in.
        vec![SubviewPlacement::new(
            bounds,
            ProposalSize::new(Some(bounds.width()), Some(bounds.height())),
        )]
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment != self.alignment {
            return None;
        }

        assert!(
            children.len() == 1,
            "HorizontalAlignmentGuideLayout expects exactly one placed child"
        );

        let child = children[0];
        Some(child.frame.x() + (self.compute)(&child.dimensions()))
    }

    fn explicit_horizontal_alignments(&self) -> Vec<HorizontalAlignment> {
        vec![self.alignment]
    }

    /// Overriding a guide changes where the child lines up, never how much space
    /// it claims, so the answer is the child's.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        children.first().copied().unwrap_or_default()
    }
}

/// A wrapper view that overrides one horizontal alignment guide.
#[must_use]
pub struct HorizontalAlignmentGuide<Content, F> {
    content: Content,
    alignment: HorizontalAlignment,
    compute: F,
}

impl<Content: View, F> HorizontalAlignmentGuide<Content, F> {
    /// Creates a new horizontal alignment-guide wrapper.
    pub const fn new(content: Content, alignment: HorizontalAlignment, compute: F) -> Self {
        Self {
            content,
            alignment,
            compute,
        }
    }
}

impl<Content, F> fmt::Debug for HorizontalAlignmentGuide<Content, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HorizontalAlignmentGuide")
            .field("alignment", &self.alignment)
            .finish_non_exhaustive()
    }
}

impl<Content, F> View for HorizontalAlignmentGuide<Content, F>
where
    Content: View + 'static,
    F: Fn(&ViewDimensions) -> f32 + 'static,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(
            HorizontalAlignmentGuideLayout {
                alignment: self.alignment,
                compute: self.compute,
            },
            (self.content,),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }
}

#[derive(Clone)]
struct VerticalAlignmentGuideLayout<F> {
    alignment: VerticalAlignment,
    compute: F,
}

impl<F> fmt::Debug for VerticalAlignmentGuideLayout<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerticalAlignmentGuideLayout")
            .field("alignment", &self.alignment)
            .finish_non_exhaustive()
    }
}

impl<F> Layout for VerticalAlignmentGuideLayout<F>
where
    F: Fn(&ViewDimensions) -> f32 + 'static,
{
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        assert!(
            children.len() == 1,
            "VerticalAlignmentGuideLayout expects exactly one child"
        );
        children[0].measure(proposal).size
    }

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
        assert!(
            children.len() == 1,
            "VerticalAlignmentGuideLayout expects exactly one child"
        );
        // Placement negotiates on both axes against the bounds: the child is
        // proposed the region it is actually placed in.
        vec![SubviewPlacement::new(
            bounds,
            ProposalSize::new(Some(bounds.width()), Some(bounds.height())),
        )]
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        if alignment != self.alignment {
            return None;
        }

        assert!(
            children.len() == 1,
            "VerticalAlignmentGuideLayout expects exactly one placed child"
        );

        let child = children[0];
        Some(child.frame.y() + (self.compute)(&child.dimensions()))
    }

    fn explicit_vertical_alignments(&self) -> Vec<VerticalAlignment> {
        vec![self.alignment]
    }

    /// Overriding a guide changes where the child lines up, never how much space
    /// it claims, so the answer is the child's.
    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        children.first().copied().unwrap_or_default()
    }
}

/// A wrapper view that overrides one vertical alignment guide.
#[must_use]
pub struct VerticalAlignmentGuide<Content, F> {
    content: Content,
    alignment: VerticalAlignment,
    compute: F,
}

impl<Content: View, F> VerticalAlignmentGuide<Content, F> {
    /// Creates a new vertical alignment-guide wrapper.
    pub const fn new(content: Content, alignment: VerticalAlignment, compute: F) -> Self {
        Self {
            content,
            alignment,
            compute,
        }
    }
}

impl<Content, F> fmt::Debug for VerticalAlignmentGuide<Content, F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerticalAlignmentGuide")
            .field("alignment", &self.alignment)
            .finish_non_exhaustive()
    }
}

impl<Content, F> View for VerticalAlignmentGuide<Content, F>
where
    Content: View + 'static,
    F: Fn(&ViewDimensions) -> f32 + 'static,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(
            VerticalAlignmentGuideLayout {
                alignment: self.alignment,
                compute: self.compute,
            },
            (self.content,),
        )
    }

    fn stretch_axis(&self) -> StretchAxis {
        self.content.stretch_axis()
    }
}

#[cfg(test)]
mod tests {
    use waterui_core::{AnyView, Environment, View, layout::Point};

    use super::*;

    struct StretchingView;

    impl View for StretchingView {
        fn body(self, _env: &Environment) -> impl View {}

        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::Horizontal
        }
    }

    struct GuidedSubview {
        size: Size,
    }

    impl SubView for GuidedSubview {
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
    fn horizontal_alignment_guide_preserves_content_stretch_axis() {
        let env = Environment::default();
        let body = HorizontalAlignmentGuide::new(
            StretchingView,
            HorizontalAlignment::Leading,
            |_dimensions: &ViewDimensions| 12.0,
        )
        .body(&env);
        let container = AnyView::new(body)
            .downcast::<FixedContainer>()
            .expect("alignment guide body should be a FixedContainer");
        let (layout, children) = container.as_parts();

        assert_eq!(children.len(), 1);
        let child_axes: Vec<StretchAxis> = children.iter().map(View::stretch_axis).collect();
        assert_eq!(layout.stretch_axis(&child_axes), StretchAxis::Horizontal);
    }

    #[test]
    fn horizontal_alignment_guide_body_exposes_override_key() {
        let env = Environment::default();
        let body = HorizontalAlignmentGuide::new(
            (),
            HorizontalAlignment::Leading,
            |dimensions: &ViewDimensions| dimensions.size.width * 0.5,
        )
        .body(&env);
        let container = AnyView::new(body)
            .downcast::<FixedContainer>()
            .expect("alignment guide body should be a FixedContainer");
        let (layout, children) = container.as_parts();

        assert_eq!(children.len(), 1);
        let child_axes: Vec<StretchAxis> = children.iter().map(View::stretch_axis).collect();
        assert_eq!(layout.stretch_axis(&child_axes), StretchAxis::None);
        assert_eq!(
            layout.explicit_horizontal_alignments(),
            vec![HorizontalAlignment::Leading]
        );

        let child = GuidedSubview {
            size: Size::new(40.0, 10.0),
        };
        let bounds = Rect::new(Point::zero(), Size::new(40.0, 10.0));
        let placed = [PlacedSubview::new(
            &child,
            SubviewPlacement::new(bounds, ProposalSize::UNSPECIFIED),
        )];
        let guide = layout
            .explicit_horizontal(HorizontalAlignment::Leading, bounds, &placed)
            .expect("horizontal guide override should be present");

        assert_eq!(guide, 20.0);
    }

    #[test]
    fn alignment_guide_wrapper_reproposes_bounds() {
        // Placement is a fresh negotiation: the wrapper's child is proposed
        // the bounds it is actually placed in — never the probe the wrapper
        // was measured under. Measured at 60x20 and placed into 100x40, the
        // child must hear 100x40.
        let child = GuidedSubview {
            size: Size::new(20.0, 10.0),
        };
        let bounds = Rect::new(Point::zero(), Size::new(100.0, 40.0));
        let probe = ProposalSize::new(Some(60.0), Some(20.0));

        let horizontal = HorizontalAlignmentGuideLayout {
            alignment: HorizontalAlignment::Leading,
            compute: |dimensions: &ViewDimensions| dimensions.size.width,
        };
        assert_eq!(
            horizontal.size_that_fits(probe, &[&child]),
            Size::new(20.0, 10.0)
        );
        assert_eq!(
            horizontal.place(bounds, probe, &[&child]),
            vec![SubviewPlacement::new(
                bounds,
                ProposalSize::new(Some(100.0), Some(40.0))
            )],
            "the horizontal guide wrapper must re-propose the bounds"
        );

        let vertical = VerticalAlignmentGuideLayout {
            alignment: VerticalAlignment::Top,
            compute: |dimensions: &ViewDimensions| dimensions.size.height,
        };
        assert_eq!(
            vertical.size_that_fits(probe, &[&child]),
            Size::new(20.0, 10.0)
        );
        assert_eq!(
            vertical.place(bounds, probe, &[&child]),
            vec![SubviewPlacement::new(
                bounds,
                ProposalSize::new(Some(100.0), Some(40.0))
            )],
            "the vertical guide wrapper must re-propose the bounds"
        );
    }

    #[test]
    fn vertical_alignment_guide_body_exposes_override_key() {
        let env = Environment::default();
        let body = VerticalAlignmentGuide::new(
            (),
            VerticalAlignment::Top,
            |dimensions: &ViewDimensions| dimensions.size.height * 0.25,
        )
        .body(&env);
        let container = AnyView::new(body)
            .downcast::<FixedContainer>()
            .expect("alignment guide body should be a FixedContainer");
        let (layout, children) = container.as_parts();

        assert_eq!(children.len(), 1);
        let child_axes: Vec<StretchAxis> = children.iter().map(View::stretch_axis).collect();
        assert_eq!(layout.stretch_axis(&child_axes), StretchAxis::None);
        assert_eq!(
            layout.explicit_vertical_alignments(),
            vec![VerticalAlignment::Top]
        );

        let child = GuidedSubview {
            size: Size::new(40.0, 20.0),
        };
        let bounds = Rect::new(Point::zero(), Size::new(40.0, 20.0));
        let placed = [PlacedSubview::new(
            &child,
            SubviewPlacement::new(bounds, ProposalSize::UNSPECIFIED),
        )];
        let guide = layout
            .explicit_vertical(VerticalAlignment::Top, bounds, &placed)
            .expect("vertical guide override should be present");

        assert_eq!(guide, 5.0);
    }
}
