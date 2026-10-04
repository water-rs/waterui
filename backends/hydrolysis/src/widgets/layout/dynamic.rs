use crate::renderer::{
    HydroNativeView, HydroState, measure_view_dimensions, measure_view_dimensions_with_proposal,
    normalize_layout_view,
};
use std::rc::Rc;
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};

/// Measures a `Dynamic` node's current content if the node still owns it, or
/// reaches the retained child through the renderer's identity registry when
/// the content has already been handed to the render pipeline.
fn measure_dynamic(
    state: &mut HydroState,
    dynamic: &Dynamic,
    env: &Environment,
    proposal: ProposalSize,
    theme: &Rc<dyn crate::engine::WidgetTheme>,
) -> ViewDimensions {
    let identity = dynamic.identity();
    state
        .measurement
        .begin_dynamic_measurement(identity, proposal);
    let measure_content = |slot: &mut Option<waterui_core::AnyView>, state: &mut HydroState| {
        slot.take().map(|content| {
            let normalized = normalize_layout_view(content, env);
            let dimensions = if proposal == ProposalSize::UNSPECIFIED {
                measure_view_dimensions(&normalized, state, env, theme)
            } else {
                measure_view_dimensions_with_proposal(&normalized, proposal, state, env, theme)
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
                Some(None) | None => match state.measurement.dynamic_node(identity) {
                    // The connected content lives in the retained
                    // `DynamicHostNode` child: measure it for the proposal
                    // actually given. Its subtree reads the content's live
                    // signals, so a size that changed in place since the last
                    // staged update — an image whose async decode just
                    // published its natural size — is answered now. The
                    // `dynamic_dimensions` cache only stands in once the
                    // registry has dropped the node (a row's retained subtree
                    // evicted offscreen still answers its last live measure).
                    Some(node) => node.borrow().measure(state, env, theme, proposal),
                    None => state
                        .measurement
                        .dynamic_dimensions(identity, proposal)
                        .unwrap_or_else(|| {
                            panic!(
                                "hydrolysis Dynamic measurement found a connected dynamic node missing from the retained registry"
                            )
                        }),
                },
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
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        measure_dynamic(
            state,
            view.as_inner(),
            env,
            ProposalSize::UNSPECIFIED,
            theme,
        )
        .size
    }

    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        measure_dynamic(state, view.as_inner(), env, proposal, theme)
    }
}
