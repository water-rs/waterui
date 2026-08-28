//! Constrains a view to a width-to-height ratio.

use alloc::{vec, vec::Vec};
use nami::{Computed, Signal, SignalExt};
use waterui_core::{AnyView, IntoSignalF32, View, layout::LayoutInvalidationCallback};

use crate::{
    Layout, Point, ProposalSize, Rect, Size, StretchAxis, SubView, container::FixedContainer,
};

/// How a view fills a ratio-constrained box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ContentMode {
    /// Shrink to fit entirely inside the offer, leaving space on the long axis.
    #[default]
    Fit,
    /// Grow to cover the offer entirely, overflowing on the long axis.
    Fill,
}

/// Layout that sizes its single child to a fixed width-to-height ratio.
#[derive(Debug, Clone)]
pub struct AspectRatioLayout {
    ratio: Computed<f32>,
    mode: ContentMode,
}

impl AspectRatioLayout {
    /// Resolves the size that honours `ratio` inside `available`.
    ///
    /// With both axes offered, `Fit` takes the smaller of the two candidate sizes
    /// and `Fill` the larger. With one axis offered the other follows from the
    /// ratio, and with neither the child's own size sets the scale.
    fn resolve(&self, available: ProposalSize, content: Size) -> Size {
        let ratio = self.ratio.get();
        assert!(
            ratio > 0.0 && ratio.is_finite(),
            "aspect ratio must be a positive, finite width-to-height ratio, got {ratio}"
        );

        match (available.width, available.height) {
            (Some(width), Some(height)) if width.is_finite() && height.is_finite() => {
                let from_width = Size::new(width, width / ratio);
                let from_height = Size::new(height * ratio, height);
                let width_fits = from_width.height <= height;
                let take_width = match self.mode {
                    ContentMode::Fit => width_fits,
                    ContentMode::Fill => !width_fits,
                };
                if take_width { from_width } else { from_height }
            }
            (Some(width), _) if width.is_finite() => Size::new(width, width / ratio),
            (_, Some(height)) if height.is_finite() => Size::new(height * ratio, height),
            _ => {
                // Unconstrained: keep the child's own scale, on whichever axis it
                // expressed one.
                if content.width > 0.0 {
                    Size::new(content.width, content.width / ratio)
                } else {
                    Size::new(content.height * ratio, content.height)
                }
            }
        }
    }
}

impl Layout for AspectRatioLayout {
    fn size_that_fits(&self, proposal: ProposalSize, children: &[&dyn SubView]) -> Size {
        let content = children
            .first()
            .map_or(Size::zero(), |child| child.measure(proposal).size);
        self.resolve(proposal, content)
    }

    fn place(&self, bounds: Rect, children: &[&dyn SubView]) -> Vec<Rect> {
        if children.is_empty() {
            return vec![];
        }
        // The child is handed the ratio-correct box, centred in the bounds so a
        // `Fit` leaves equal slack on both sides and a `Fill` overflows evenly.
        let size = self.resolve(
            ProposalSize::new(Some(bounds.width()), Some(bounds.height())),
            *bounds.size(),
        );
        let origin = Point::new(
            bounds.x() + (bounds.width() - size.width) * 0.5,
            bounds.y() + (bounds.height() - size.height) * 0.5,
        );
        vec![Rect::new(origin, size)]
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        // The ratio decides the shape, never how much space is claimed: that is
        // still the child's to ask for.
        StretchAxis::None
    }

    fn watch_invalidation(
        &self,
        invalidate: LayoutInvalidationCallback,
    ) -> Vec<nami::watcher::BoxWatcherGuard> {
        vec![self.ratio.watch(move |_| invalidate())]
    }
}

/// A view constrained to a width-to-height ratio.
#[derive(Debug)]
pub struct AspectRatio {
    layout: AspectRatioLayout,
    content: AnyView,
}

impl AspectRatio {
    /// Constrains `content` to `ratio`, expressed as width divided by height.
    ///
    /// # Panics
    ///
    /// Measuring panics if the ratio is not positive and finite.
    #[must_use]
    pub fn new(content: impl View, ratio: impl IntoSignalF32 + 'static, mode: ContentMode) -> Self {
        Self {
            layout: AspectRatioLayout {
                ratio: ratio.into_signal_f32().computed(),
                mode,
            },
            content: AnyView::new(content),
        }
    }
}

impl View for AspectRatio {
    fn body(self, _env: &waterui_core::Environment) -> impl View {
        FixedContainer::new(self.layout, (self.content,))
    }
}

/// Constrains `content` to `ratio`, shrinking to fit inside what is offered.
pub fn aspect_ratio(content: impl View, ratio: impl IntoSignalF32 + 'static) -> AspectRatio {
    AspectRatio::new(content, ratio, ContentMode::Fit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ViewDimensions;

    struct Content(Size);

    impl SubView for Content {
        fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
            ViewDimensions::new(self.0)
        }
        fn stretch_axis(&self) -> StretchAxis {
            StretchAxis::None
        }
        fn priority(&self) -> i32 {
            0
        }
    }

    fn layout(mode: ContentMode) -> AspectRatioLayout {
        AspectRatioLayout {
            ratio: Computed::constant(2.0),
            mode,
        }
    }

    #[test]
    fn fit_stays_inside_a_wider_offer() {
        // 2:1 inside a 300x100 box fits by height: 200x100, not 300x150.
        let child = Content(Size::new(10.0, 10.0));
        let size =
            layout(ContentMode::Fit).size_that_fits(ProposalSize::new(300.0, 100.0), &[&child]);
        assert!((size.width - 200.0).abs() < 0.01, "got {size:?}");
        assert!((size.height - 100.0).abs() < 0.01, "got {size:?}");
    }

    #[test]
    fn fill_covers_a_wider_offer() {
        let child = Content(Size::new(10.0, 10.0));
        let size =
            layout(ContentMode::Fill).size_that_fits(ProposalSize::new(300.0, 100.0), &[&child]);
        assert!((size.width - 300.0).abs() < 0.01, "got {size:?}");
        assert!((size.height - 150.0).abs() < 0.01, "got {size:?}");
    }

    #[test]
    fn one_offered_axis_decides_the_other() {
        let child = Content(Size::new(10.0, 10.0));
        let size =
            layout(ContentMode::Fit).size_that_fits(ProposalSize::new(Some(80.0), None), &[&child]);
        assert!((size.height - 40.0).abs() < 0.01, "got {size:?}");
    }

    #[test]
    fn the_child_sets_the_scale_when_nothing_is_offered() {
        let child = Content(Size::new(60.0, 999.0));
        let size = layout(ContentMode::Fit).size_that_fits(ProposalSize::UNSPECIFIED, &[&child]);
        assert!((size.width - 60.0).abs() < 0.01, "got {size:?}");
        assert!((size.height - 30.0).abs() < 0.01, "got {size:?}");
    }

    #[test]
    fn the_child_is_centred_in_the_bounds_it_does_not_fill() {
        let child = Content(Size::new(10.0, 10.0));
        let rects = layout(ContentMode::Fit)
            .place(Rect::new(Point::zero(), Size::new(300.0, 100.0)), &[&child]);
        assert!(
            (rects[0].width() - 200.0).abs() < 0.01,
            "got {:?}",
            rects[0]
        );
        assert!((rects[0].x() - 50.0).abs() < 0.01, "got {:?}", rects[0]);
    }
}
