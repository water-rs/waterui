use crate::renderer::{
    HydroNativeView, HydroState, measure_view_dimensions, measure_view_dimensions_with_proposal,
    normalize_layout_view,
};
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};

/// Measures a `Dynamic` node's current content if the node still owns it, or
/// falls back to the renderer's per-node dimension caches when the content has
/// already been handed to the render pipeline.
fn measure_dynamic(
    state: &mut HydroState,
    dynamic: &Dynamic,
    env: &Environment,
    proposal: ProposalSize,
) -> ViewDimensions {
    let identity = dynamic.identity();
    state
        .measurement
        .begin_dynamic_measurement(identity, proposal);
    let measure_content = |slot: &mut Option<waterui_core::AnyView>, state: &mut HydroState| {
        slot.take().map(|content| {
            let normalized = normalize_layout_view(content, env);
            let dimensions = if proposal == ProposalSize::UNSPECIFIED {
                measure_view_dimensions(&normalized, state, env)
            } else {
                measure_view_dimensions_with_proposal(&normalized, proposal, state, env)
            };
            *slot = Some(normalized);
            dimensions
        })
    };
    let initial = dynamic.with_unconnected_view_mut(|slot| measure_content(slot, state));
    let dimensions = match initial {
        Some(Some(dimensions)) => dimensions,
        Some(None) => {
            panic!("hydrolysis Dynamic measurement requires an initial view before layout")
        }
        None => {
            match dynamic.with_connected_pending_view_mut(|slot| measure_content(slot, state)) {
                Some(Some(dimensions)) => dimensions,
                Some(None) | None => state
                    .measurement
                    .dynamic_dimensions(identity, proposal)
                    .or_else(|| state.measurement.dynamic_intrinsic(identity))
                    .unwrap_or_else(|| {
                        panic!("hydrolysis Dynamic intrinsic cache miss for connected dynamic node")
                    }),
            }
        }
    };
    state
        .measurement
        .store_dynamic_dimensions(identity, proposal, dimensions.clone());
    state
        .measurement
        .finish_dynamic_measurement(identity, "after dynamic measurement");
    dimensions
}

impl HydroNativeView for Native<Dynamic> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        measure_dynamic(state, view.as_inner(), env, ProposalSize::UNSPECIFIED).size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        measure_dynamic(state, view.as_inner(), env, proposal)
    }
}
