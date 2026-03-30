use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, RenderContext, WidgetRenderContext,
    measure_view_dimensions, normalize_layout_view,
};
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};

impl HydroNativeView for Native<Dynamic> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_dynamic(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let dynamic = view.as_inner();
        let identity = dynamic.identity();
        if let Some(dimensions) = state.dynamic_intrinsic_cache.get(&identity) {
            return dimensions.size;
        }
        let initial = dynamic.with_unconnected_view_mut(|slot| {
            slot.take().map(|content| {
                let normalized = normalize_layout_view(content, env);
                let dimensions = measure_view_dimensions(&normalized, state, env);
                *slot = Some(normalized);
                dimensions
            })
        });
        let dimensions = match initial {
            Some(Some(dimensions)) => dimensions,
            Some(None) => {
                panic!("hydrolysis Dynamic intrinsic requires an initial view before layout")
            }
            None => state
                .dynamic_intrinsic_cache
                .get(&identity)
                .cloned()
                .unwrap_or_else(|| {
                    panic!("hydrolysis Dynamic intrinsic cache miss for connected dynamic node")
                }),
        };
        state
            .dynamic_intrinsic_cache
            .insert(identity, dimensions.clone());
        dimensions.size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        let dynamic = view.as_inner();
        let identity = dynamic.identity();
        if proposal == ProposalSize::UNSPECIFIED
            && let Some(dimensions) = state.dynamic_intrinsic_cache.get(&identity)
        {
            return dimensions.clone();
        }

        let initial = dynamic.with_unconnected_view_mut(|slot| {
            slot.take().map(|content| {
                let normalized = normalize_layout_view(content, env);
                let dimensions = crate::renderer::measure_view_dimensions_with_proposal(
                    &normalized,
                    proposal,
                    state,
                    env,
                );
                *slot = Some(normalized);
                dimensions
            })
        });
        let dimensions = match initial {
            Some(Some(dimensions)) => dimensions,
            Some(None) => {
                panic!("hydrolysis Dynamic dimensions requires an initial view before layout")
            }
            None => state
                .dynamic_intrinsic_cache
                .get(&identity)
                .cloned()
                .unwrap_or_else(|| {
                    panic!("hydrolysis Dynamic dimensions cache miss for connected dynamic node")
                }),
        };
        state
            .dynamic_intrinsic_cache
            .insert(identity, dimensions.clone());
        dimensions
    }
}
