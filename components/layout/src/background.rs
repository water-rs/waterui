//! Background layout for rendering content behind a view.
//!
//! `background` is the inverse of `overlay` - the background view renders
//! behind the content, and the content determines the layout size.
//!
//! # Example
//!
//! ```rust,ignore
//! use waterui::prelude::*;
//!
//! // Gradient behind text
//! text!("Hello").background(gradient_view)
//!
//! // Using the layout directly
//! background(content, background_view)
//! ```

use core::fmt;

use alloc::vec::Vec;
use waterui_core::View;

use crate::{
    HorizontalAlignment, Layout, PlacedSubview, ProposalSize, Rect, Size, StretchAxis, SubView,
    VerticalAlignment, container::FixedContainer,
};

/// Layout used by [`BackgroundView`] to render a background behind content.
///
/// The content child determines the size, the background fills the bounds.
/// Children order: `[background, content]`
#[derive(Debug, Clone, Copy, Default)]
pub struct BackgroundLayout {
    stretch_axis: StretchAxis,
}

impl Layout for BackgroundLayout {
    fn stretch_axis(&self) -> StretchAxis {
        self.stretch_axis
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        // Size is driven entirely by the content child (index 1).
        // Background just fills whatever size content needs.
        // children[0] = background, children[1] = content
        children
            .get(1)
            .expect("BackgroundLayout requires a content child at index 1")
            .measure(proposal)
            .size
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        assert!(
            children.len() >= 2,
            "BackgroundLayout requires at least 2 children: background and content"
        );

        let mut placements = Vec::with_capacity(children.len());

        // Background (index 0) fills the entire bounds
        placements.push(Rect::new(
            bounds.origin(),
            Size::new(bounds.width(), bounds.height()),
        ));

        // Content (index 1) also fills the bounds
        placements.push(Rect::new(
            bounds.origin(),
            Size::new(bounds.width(), bounds.height()),
        ));

        placements
    }

    fn explicit_horizontal(
        &self,
        alignment: HorizontalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .get(1)
            .and_then(|child| child.explicit_horizontal(alignment))
    }

    fn explicit_vertical(
        &self,
        alignment: VerticalAlignment,
        _bounds: Rect,
        children: &[PlacedSubview<'_>],
    ) -> Option<f32> {
        children
            .get(1)
            .and_then(|child| child.explicit_vertical(alignment))
    }
}

/// A view that renders a `background` behind the `content`.
///
/// The content determines the layout size, and the background fills the bounds.
/// The background renders first (behind), then content renders on top.
pub struct BackgroundView<Content, Bg> {
    content: Content,
    background: Bg,
}

impl<Content, Bg> BackgroundView<Content, Bg> {
    /// Creates a new background view with the provided content and background.
    #[must_use]
    pub const fn new(content: Content, background: Bg) -> Self {
        Self {
            content,
            background,
        }
    }
}

impl<Content, Bg> fmt::Debug for BackgroundView<Content, Bg> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BackgroundView").finish_non_exhaustive()
    }
}

impl<Content, Bg> View for BackgroundView<Content, Bg>
where
    Content: View + 'static,
    Bg: View + 'static,
{
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        let Self {
            content,
            background,
        } = self;
        let stretch_axis = content.stretch_axis();

        // Background is first (renders behind), content is second (renders on top)
        FixedContainer::new(BackgroundLayout { stretch_axis }, (background, content))
    }
}

/// Convenience constructor for creating a [`BackgroundView`].
#[must_use]
pub const fn background<Content, Bg>(
    content: Content,
    background: Bg,
) -> BackgroundView<Content, Bg> {
    BackgroundView::new(content, background)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use waterui_core::layout::Point;
    use waterui_core::layout::ViewDimensions;

    use super::*;
    use alloc::vec;

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
    fn test_background_size_from_content() {
        let layout = BackgroundLayout::default();

        let mut bg = MockSubView {
            size: Size::new(200.0, 200.0),
        };
        let mut content = MockSubView {
            size: Size::new(100.0, 50.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut bg, &mut content];
        let size = layout.size_that_fits(ProposalSize::UNSPECIFIED, &children);

        // Size comes from content child (index 1)
        assert_eq!(size.width, 100.0);
        assert_eq!(size.height, 50.0);
    }

    #[test]
    fn test_background_fills_bounds() {
        let layout = BackgroundLayout::default();

        let mut bg = MockSubView {
            size: Size::new(50.0, 50.0),
        };
        let mut content = MockSubView {
            size: Size::new(100.0, 100.0),
        };

        let children: Vec<&dyn SubView> = vec![&mut bg, &mut content];
        let bounds = Rect::new(Point::new(0.0, 0.0), Size::new(100.0, 100.0));
        let rects = layout.place(bounds, &children);

        // Both fill the bounds
        assert_eq!(rects[0].width(), 100.0);
        assert_eq!(rects[0].height(), 100.0);
        assert_eq!(rects[1].width(), 100.0);
        assert_eq!(rects[1].height(), 100.0);
    }

    #[test]
    fn test_background_layout_preserves_content_stretch_axis() {
        let layout = BackgroundLayout {
            stretch_axis: StretchAxis::Horizontal,
        };
        assert_eq!(layout.stretch_axis(), StretchAxis::Horizontal);
    }
}
