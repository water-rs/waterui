//! Constrains a view to a width-to-height ratio.

use alloc::{vec, vec::Vec};
use nami::{Computed, Signal, SignalExt};
use waterui_contract::contract_violation;
use waterui_core::{AnyView, IntoSignalF32, View, layout::LayoutInvalidationCallback};

use crate::{
    Layout, LayoutError, PlacementInput, Point, ProposalSize, Rect, Size, StretchAxis, SubView,
    SubviewPlacement, UnitPoint, container::FixedContainer,
};

/// How a view fills a ratio-constrained box.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[expect(
    clippy::exhaustive_enums,
    reason = "closed: content fits inside or fills the ratio box"
)]
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
        let mut ratio = self.ratio.snapshot();
        if !(ratio > 0.0 && ratio.is_finite()) {
            contract_violation!(
                "aspect ratio must be a positive, finite width-to-height ratio, got {ratio}; laying out with ratio 1.0"
            );
            ratio = 1.0;
        }

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

    fn place(
        &self,
        bounds: Rect,
        _proposal: ProposalSize,
        children: &[&dyn SubView],
    ) -> Vec<SubviewPlacement> {
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
        vec![SubviewPlacement::new(
            Rect::new(origin, size),
            ProposalSize::new(size.width, size.height),
        )]
    }

    /// The answer centred in the bounds. It is the ratio box for the
    /// incoming proposal, which need not be the ratio box `place` resolves
    /// against the bounds.
    fn resolve_extent(
        &self,
        input: PlacementInput,
        _children: &[&dyn SubView],
    ) -> Result<Rect, LayoutError> {
        Ok(input.bounds.align(input.extent()?, UnitPoint::CENTER))
    }

    fn stretch_axis(&self, children: &[StretchAxis]) -> StretchAxis {
        // The ratio decides the shape, never how much space is claimed: that is
        // still the child's to ask for — so forward its answer.
        children.first().copied().unwrap_or_default()
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
    /// A ratio that is not positive and finite is a contract violation: a
    /// debug build panics when it is measured, a release build logs it and
    /// lays out with a ratio of 1.0.
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

    /// Resolves to `FixedContainer` over the same layout and single child;
    /// reports what that container would.
    fn stretch_axis(&self) -> StretchAxis {
        self.layout.stretch_axis(&[self.content.stretch_axis()])
    }
}

/// Constrains `content` to `ratio`, shrinking to fit inside what is offered.
pub fn aspect_ratio(content: impl View, ratio: impl IntoSignalF32 + 'static) -> AspectRatio {
    AspectRatio::new(content, ratio, ContentMode::Fit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LayoutError, Rect, ResolvedExtent, ViewDimensions};

    struct Content(Size);

    impl SubView for Content {
        fn resolve_extent(
            &self,
            bounds: Rect,
            proposal: ProposalSize,
        ) -> Result<ResolvedExtent, LayoutError> {
            ResolvedExtent::centred(bounds, self.measure(proposal).size)
        }
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
    #[cfg_attr(
        debug_assertions,
        should_panic(expected = "aspect ratio must be a positive, finite")
    )]
    fn an_invalid_ratio_lays_out_as_a_square() {
        for ratio in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            let layout = AspectRatioLayout {
                ratio: Computed::constant(ratio),
                mode: ContentMode::Fit,
            };
            let child = Content(Size::new(10.0, 10.0));
            let size = layout.size_that_fits(ProposalSize::new(300.0, 100.0), &[&child]);
            assert!(
                (size.width - 100.0).abs() < 0.01 && (size.height - 100.0).abs() < 0.01,
                "ratio {ratio} must lay out as 1.0, got {size:?}"
            );
        }
    }

    #[test]
    fn the_child_is_centred_in_the_bounds_it_does_not_fill_bounded_proposal() {
        let child = Content(Size::new(10.0, 10.0));
        let bounds = Rect::new(Point::zero(), Size::new(300.0, 100.0));
        let proposal = ProposalSize::new(Some(bounds.width()), Some(bounds.height()));
        let placements = layout(ContentMode::Fit).place(bounds, proposal, &[&child]);
        assert!(
            (placements[0].frame.width() - 200.0).abs() < 0.01,
            "got {:?}",
            placements[0].frame
        );
        assert!(
            (placements[0].frame.x() - 50.0).abs() < 0.01,
            "got {:?}",
            placements[0].frame
        );
    }
}
