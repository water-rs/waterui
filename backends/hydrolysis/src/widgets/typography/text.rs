use crate::renderer::{HydroNativeView, HydroState, HydrolysisRenderer};
use nami::Signal;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_text::TextConfig;

impl HydroNativeView for Native<TextConfig> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.get(),
            view.as_inner().paragraph_alignment.get(),
            env,
            None,
            None,
        )
        .size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.get(),
            view.as_inner().paragraph_alignment.get(),
            env,
            proposal.width,
            None,
        )
    }
}
