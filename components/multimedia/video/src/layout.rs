//! The leaf contract a `Fit` video answers by, for the realizations that
//! compose their picture from WaterUI views.
//!
//! A `Fit` video takes the width it is proposed and the height its source's
//! aspect ratio gives at that width, whatever height is on offer: it stretches
//! horizontally only, like a resizable image that keeps its shape. Until the
//! source reports its size the ratio is [`DEFAULT_ASPECT`]. `Fill` and
//! `Stretch` fill the proposal on both axes and need no layout of their own.

use nami::{Computed, Signal};
use waterui_core::layout::LayoutInvalidationCallback;
use waterui_core::{AnyView, View};
use waterui_layout::container::FixedContainer;
use waterui_layout::{Layout, ProposalSize, Rect, Size, StretchAxis, SubView, SubviewPlacement};

/// The width-to-height ratio a video answers by before its source reports
/// its size: 16:9.
pub const DEFAULT_ASPECT: f32 = 16.0 / 9.0;

/// The width a `Fit` video answers to an unspecified width proposal before
/// its source reports its size.
const FALLBACK_WIDTH: f32 = 320.0;

/// Sizes a `Fit` video's single child: the proposed width, and the height
/// `aspect` gives at that width.
#[derive(Debug, Clone)]
pub struct FitVideoLayout {
    aspect: Computed<f32>,
    natural_width: Computed<Option<f32>>,
}

impl FitVideoLayout {
    fn size(&self, proposal: ProposalSize) -> Size {
        let aspect = self.aspect.snapshot();
        assert!(
            aspect > 0.0 && aspect.is_finite(),
            "a video's aspect ratio must be positive and finite, got {aspect}"
        );
        let width = proposal
            .width
            .filter(|width| width.is_finite())
            .unwrap_or_else(|| self.natural_width.snapshot().unwrap_or(FALLBACK_WIDTH));
        Size::new(width, width / aspect)
    }
}

impl Layout for FitVideoLayout {
    fn size_that_fits(&self, proposal: ProposalSize, _children: &[&dyn SubView]) -> Size {
        self.size(proposal)
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
        let size = self.size(ProposalSize::new(Some(bounds.width()), None));
        vec![SubviewPlacement::new(
            Rect::new(bounds.origin(), size),
            ProposalSize::new(size.width, size.height),
        )]
    }

    fn stretch_axis(&self, _children: &[StretchAxis]) -> StretchAxis {
        StretchAxis::Horizontal
    }

    fn watch_invalidation(
        &self,
        invalidate: LayoutInvalidationCallback,
    ) -> Vec<nami::watcher::BoxWatcherGuard> {
        let natural = invalidate.clone();
        vec![
            self.aspect.watch(move |_| invalidate()),
            self.natural_width.watch(move |_| natural()),
        ]
    }
}

/// `picture` laid out as a `Fit` video: the proposed width and the height
/// `aspect` gives at that width. `natural_width` is the source's own width,
/// answered to an unspecified width proposal once the source reports it.
pub fn fit_video(
    picture: impl View,
    aspect: Computed<f32>,
    natural_width: Computed<Option<f32>>,
) -> impl View {
    FixedContainer::new(
        FitVideoLayout {
            aspect,
            natural_width,
        },
        (AnyView::new(picture),),
    )
}
