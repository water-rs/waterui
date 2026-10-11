//! The `spacer` leaf: `Native<Spacer>` rendered as an invisible view that
//! expands on the enclosing stack's main axis.
//!
//! Mirrors `WuiSpacer`: the view draws nothing, answers
//! `StretchAxis::MainAxis` and `Spacer::DEFAULT_LAYOUT_PRIORITY`, and its
//! measure is the minimum length — which axis that is resolves from the
//! `stack::Axis` the enclosing stack injects into the environment
//! (`HStack`/`VStack` wrap their children in `with(.., Axis)`), so the leaf
//! answers `min_length` on the main axis and zero on the cross axis whatever
//! the proposal, and claims nothing under an axis-less container — the
//! layout-spec §5/§6 leaf contract. `WuiSpacer` never read `min_length`, so a
//! `spacer_min(n)` reserved nothing under compression; reading the payload is
//! the one deliberate difference.

use waterui::layout::{Spacer, stack::Axis};
use waterui_core::layout::{ProposalSize, Size, StretchAxis, SubView, ViewDimensions};

use crate::contract::NativeLeaf;
use crate::dispatch::Dispatcher;

#[cfg(target_os = "macos")]
use cocoa_ui::appkit::HostView;
#[cfg(target_os = "ios")]
use cocoa_ui::uikit::HostView;

/// A spacer's layout face: `min_length` on the enclosing stack's main axis,
/// zero elsewhere, whatever the proposal — the floor the stack keeps under
/// compression; expansion happens at placement because `stretch_axis`
/// resolves `MainAxis` against the stack's own axis.
struct SpacerSubView {
    min_length: f32,
    axis: Option<Axis>,
}

impl SubView for SpacerSubView {
    fn measure(&self, _proposal: ProposalSize) -> ViewDimensions {
        ViewDimensions::new(match self.axis {
            Some(Axis::Horizontal) => Size::new(self.min_length, 0.0),
            Some(Axis::Vertical) => Size::new(0.0, self.min_length),
            _ => Size::zero(),
        })
    }

    fn stretch_axis(&self) -> StretchAxis {
        StretchAxis::MainAxis
    }

    fn priority(&self) -> i32 {
        Spacer::DEFAULT_LAYOUT_PRIORITY
    }
}

/// Installs the `spacer` handler on the dispatcher: `Native<Spacer>` maps to a
/// transparent host view whose layout face is the stack's flexible gap.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_native::<Spacer>(|spacer, ctx| {
        let view = HostView::new(ctx.mtm(), cocoa_ui::Rect::ZERO);
        NativeLeaf::new(
            &*view,
            SpacerSubView {
                min_length: spacer.min_length(),
                axis: ctx.env().get::<Axis>().copied(),
            },
        )
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subview(min_length: f32, axis: Option<Axis>) -> SpacerSubView {
        SpacerSubView { min_length, axis }
    }

    /// The leaf's measured floor is `min_length` on the stack's main axis and
    /// zero on the cross axis — under every proposal (§6): the offer is the
    /// stack's to give at placement, never the spacer's to report.
    #[test]
    fn measure_answers_min_length_on_the_main_axis_only() {
        let proposals = [
            ProposalSize::UNSPECIFIED,
            ProposalSize::ZERO,
            ProposalSize::INFINITY,
            ProposalSize::new(Some(320.0), Some(8.0)),
            ProposalSize::new(Some(0.0), None),
        ];
        for proposal in proposals {
            let horizontal = subview(20.0, Some(Axis::Horizontal)).measure(proposal);
            assert_eq!(horizontal.size, Size::new(20.0, 0.0));
            let vertical = subview(20.0, Some(Axis::Vertical)).measure(proposal);
            assert_eq!(vertical.size, Size::new(0.0, 20.0));
        }
    }

    /// Under no `Axis` — a `ZStack`, a frame, the window root — a spacer
    /// claims nothing: `axisless_stretch_union` already ignores `MainAxis`,
    /// and the measured answer is zero.
    #[test]
    fn measure_without_an_axis_is_zero() {
        let dims = subview(20.0, None).measure(ProposalSize::new(Some(100.0), Some(100.0)));
        assert_eq!(dims.size, Size::zero());
    }

    /// The default `spacer()` is `min_length = 0`, so the axis-aware answer
    /// reduces to the old leaf's `.zero` on every proposal.
    #[test]
    fn flexible_spacer_measures_zero() {
        let dims = subview(0.0, Some(Axis::Horizontal)).measure(ProposalSize::UNSPECIFIED);
        assert_eq!(dims.size, Size::zero());
    }

    /// `MainAxis` is the view's declared stretch and `i32::MIN` the band a
    /// flexible gap sits in — below ordinary content, so it receives the
    /// space remaining after every sibling's reported extent.
    #[test]
    fn stretch_and_priority_are_the_spacer_contract() {
        let leaf = subview(20.0, Some(Axis::Horizontal));
        assert_eq!(leaf.stretch_axis(), StretchAxis::MainAxis);
        assert_eq!(leaf.priority(), i32::MIN);
        assert!(!leaf.is_empty());
    }
}
