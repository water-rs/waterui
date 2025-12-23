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

use alloc::{vec, vec::Vec};
use waterui_core::View;

use crate::{
    Layout, ProposalSize, Rect, Size, StretchAxis, SubView, container::FixedContainer,
};

/// Layout used by [`BackgroundView`] to render a background behind content.
///
/// The content child determines the size, the background fills the bounds.
/// Children order: `[background, content]`
#[derive(Debug, Clone, Copy, Default)]
pub struct BackgroundLayout;

impl Layout for BackgroundLayout {
    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::Both
    }

    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        // Size is driven by the content child (index 1).
        // children[0] = background, children[1] = content
        let content_size = children
            .get(1)
            .map_or(Size::zero(), |c| c.size_that_fits(proposal));

        let content_width = if content_size.width.is_finite() && content_size.width > 0.0 {
            content_size.width
        } else {
            proposal.width.unwrap_or(0.0)
        };

        let content_height = if content_size.height.is_finite() && content_size.height > 0.0 {
            content_size.height
        } else {
            proposal.height.unwrap_or(0.0)
        };

        let width = proposal.width.unwrap_or(content_width);
        let height = proposal.height.unwrap_or(content_height);

        Size::new(width.max(0.0), height.max(0.0))
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }

        let mut placements = Vec::with_capacity(children.len());

        // Background (index 0) fills the entire bounds
        if children.first().is_some() {
            placements.push(Rect::new(
                bounds.origin(),
                Size::new(bounds.width(), bounds.height()),
            ));
        }

        // Content (index 1) also fills the bounds
        if children.get(1).is_some() {
            placements.push(Rect::new(
                bounds.origin(),
                Size::new(bounds.width(), bounds.height()),
            ));
        }

        placements
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
        Self { content, background }
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
        // Background is first (renders behind), content is second (renders on top)
        FixedContainer::new(BackgroundLayout, (self.background, self.content))
    }
}

/// Convenience constructor for creating a [`BackgroundView`].
#[must_use]
pub const fn background<Content, Bg>(content: Content, background: Bg) -> BackgroundView<Content, Bg> {
    BackgroundView::new(content, background)
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

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
    fn test_background_size_from_content() {
        let layout = BackgroundLayout;

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
        let layout = BackgroundLayout;

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
}
