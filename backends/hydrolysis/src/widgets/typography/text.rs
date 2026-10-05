use core::num::NonZeroUsize;
use std::rc::Rc;

use crate::renderer::{HydroNativeView, HydroState, HydrolysisRenderer};
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_text::TextConfig;

impl HydroNativeView for Native<TextConfig> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        let content = state.measure_signal(&view.as_inner().content);
        let alignment = state.measure_signal(&view.as_inner().paragraph_alignment);
        HydrolysisRenderer::measure_text_dimensions(
            state,
            content,
            alignment,
            env,
            None,
            view.as_inner().line_limit.map(NonZeroUsize::get),
        )
        .size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let content = state.measure_signal(&view.as_inner().content);
        let alignment = state.measure_signal(&view.as_inner().paragraph_alignment);
        HydrolysisRenderer::measure_text_dimensions(
            state,
            content,
            alignment,
            env,
            proposal.width,
            view.as_inner().line_limit.map(NonZeroUsize::get),
        )
    }
}
