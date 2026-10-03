use core::num::NonZeroUsize;
use std::rc::Rc;

use crate::renderer::{HydroNativeView, HydroState, HydrolysisRenderer};
use nami::Signal;
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
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.snapshot(),
            view.as_inner().paragraph_alignment.snapshot(),
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
        HydrolysisRenderer::measure_text_dimensions(
            state,
            view.as_inner().content.snapshot(),
            view.as_inner().paragraph_alignment.snapshot(),
            env,
            proposal.width,
            view.as_inner().line_limit.map(NonZeroUsize::get),
        )
    }
}
