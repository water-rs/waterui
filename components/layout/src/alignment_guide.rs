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
    VerticalAlignment, ViewDimensions, container::FixedContainer,
};

#[derive(Clone)]
struct HorizontalAlignmentGuideLayout<F> {
    alignment: HorizontalAlignment,
    compute: F,
    stretch_axis: StretchAxis,
}

impl<F> fmt::Debug for HorizontalAlignmentGuideLayout<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HorizontalAlignmentGuideLayout")
            .field("alignment", &self.alignment)
            .field("stretch_axis", &self.stretch_axis)
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

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        assert!(
            children.len() == 1,
            "HorizontalAlignmentGuideLayout expects exactly one child"
        );
        vec![bounds]
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

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }
}

/// A wrapper view that overrides one horizontal alignment guide.
#[must_use]
pub struct HorizontalAlignmentGuide<Content, F> {
    content: Content,
    alignment: HorizontalAlignment,
    compute: F,
}

impl<Content, F> HorizontalAlignmentGuide<Content, F> {
    /// Creates a new horizontal alignment-guide wrapper.
    #[must_use]
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
        let stretch_axis = self.content.stretch_axis();
        FixedContainer::new(
            HorizontalAlignmentGuideLayout {
                alignment: self.alignment,
                compute: self.compute,
                stretch_axis,
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
    stretch_axis: StretchAxis,
}

impl<F> fmt::Debug for VerticalAlignmentGuideLayout<F> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VerticalAlignmentGuideLayout")
            .field("alignment", &self.alignment)
            .field("stretch_axis", &self.stretch_axis)
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

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        assert!(
            children.len() == 1,
            "VerticalAlignmentGuideLayout expects exactly one child"
        );
        vec![bounds]
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

    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }
}

/// A wrapper view that overrides one vertical alignment guide.
#[must_use]
pub struct VerticalAlignmentGuide<Content, F> {
    content: Content,
    alignment: VerticalAlignment,
    compute: F,
}

impl<Content, F> VerticalAlignmentGuide<Content, F> {
    /// Creates a new vertical alignment-guide wrapper.
    #[must_use]
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
        let stretch_axis = self.content.stretch_axis();
        FixedContainer::new(
            VerticalAlignmentGuideLayout {
                alignment: self.alignment,
                compute: self.compute,
                stretch_axis,
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
        fn body(self, _env: &Environment) -> impl View {
            ()
        }

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
        assert_eq!(layout.stretch_axis(), StretchAxis::Horizontal);
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
        assert_eq!(layout.stretch_axis(), StretchAxis::None);
        assert_eq!(
            layout.explicit_horizontal_alignments(),
            vec![HorizontalAlignment::Leading]
        );

        let child = GuidedSubview {
            size: Size::new(40.0, 10.0),
        };
        let bounds = Rect::new(Point::zero(), Size::new(40.0, 10.0));
        let placed = [PlacedSubview::new(&child, bounds)];
        let guide = layout
            .explicit_horizontal(HorizontalAlignment::Leading, bounds, &placed)
            .expect("horizontal guide override should be present");

        assert_eq!(guide, 20.0);
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
        assert_eq!(layout.stretch_axis(), StretchAxis::None);
        assert_eq!(
            layout.explicit_vertical_alignments(),
            vec![VerticalAlignment::Top]
        );

        let child = GuidedSubview {
            size: Size::new(40.0, 20.0),
        };
        let bounds = Rect::new(Point::zero(), Size::new(40.0, 20.0));
        let placed = [PlacedSubview::new(&child, bounds)];
        let guide = layout
            .explicit_vertical(VerticalAlignment::Top, bounds, &placed)
            .expect("vertical guide override should be present");

        assert_eq!(guide, 5.0);
    }
}
