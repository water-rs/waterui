use crate::renderer::lazy::{LazyStackAxisConfig, lazy_stack_axis_config};
use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, WidgetRenderContext,
    estimate_layout_intrinsic, measure_layout_dimensions, measure_view_intrinsic,
    normalize_layout_view,
};
use nami::Signal;
use waterui::views::Views;
use waterui_core::layout::{ProposalSize, Size as LayoutSize};
use waterui_core::{Environment, Native};
use waterui_layout::container::{FixedContainer, LazyContainer};

impl HydroNativeView for Native<FixedContainer> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_fixed_container(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        estimate_layout_intrinsic(layout, children.iter(), state, env)
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> waterui_core::layout::ViewDimensions {
        let (layout, children) = view.as_inner().as_parts();
        measure_layout_dimensions(layout, children.iter(), proposal, state, env)
    }
}

impl HydroNativeView for Native<LazyContainer> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_lazy_container(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let (layout, children) = view.as_inner().as_parts();
        let child_count = children.len().get();
        if child_count == 0 {
            return LayoutSize::zero();
        }
        let sample = children
            .get_view(0)
            .map(|view| normalize_layout_view(view, env))
            .map(|view| measure_view_intrinsic(&view, state, env))
            .unwrap_or_else(|| panic!("LazyContainer failed to materialize child at index 0"));
        let count = child_count as f64;
        match lazy_stack_axis_config(layout) {
            LazyStackAxisConfig::Vertical { spacing, .. } => {
                let width = f64::from(sample.width);
                let height = f64::from(sample.height) * count + spacing * (count - 1.0).max(0.0);
                LayoutSize::new(width as f32, height as f32)
            }
            LazyStackAxisConfig::Horizontal { spacing, .. } => {
                let width = f64::from(sample.width) * count + spacing * (count - 1.0).max(0.0);
                let height = f64::from(sample.height);
                LayoutSize::new(width as f32, height as f32)
            }
        }
    }
}
