use crate::renderer::{
    HydroNativeView, HydroState, HydrolysisRenderer, WidgetRenderContext, measure_view_dimensions,
    normalize_layout_view,
};
use waterui_core::dynamic::Dynamic;
use waterui_core::layout::{ProposalSize, Size as LayoutSize, ViewDimensions};
use waterui_core::{Environment, Native};

fn push_dynamic_measurement(state: &mut HydroState, identity: usize, proposal: ProposalSize) {
    if let Some((_, active_proposal)) = state
        .dynamic_measurement_stack
        .iter()
        .find(|(active_identity, _)| *active_identity == identity)
    {
        panic!(
            "hydrolysis re-entered Dynamic measurement for node {identity} with proposal {:?} while already measuring {:?}",
            proposal, active_proposal
        );
    }
    state.dynamic_measurement_stack.push((identity, proposal));
}

fn pop_dynamic_measurement(state: &mut HydroState, identity: usize, phase: &str) {
    let popped = state
        .dynamic_measurement_stack
        .pop()
        .expect("hydrolysis dynamic measurement stack underflow");
    assert!(
        popped.0 == identity,
        "hydrolysis dynamic measurement stack corrupted {phase}"
    );
}

fn proposal_cache_key(
    identity: usize,
    proposal: ProposalSize,
) -> (usize, Option<u32>, Option<u32>) {
    (
        identity,
        proposal.width.map(f32::to_bits),
        proposal.height.map(f32::to_bits),
    )
}

impl HydroNativeView for Native<Dynamic> {
    fn render(ctx: &mut WidgetRenderContext<'_>, view: Self, env: &Environment) {
        let render_ctx = ctx.render_context();
        HydrolysisRenderer::render_dynamic(ctx.renderer_mut(), render_ctx, view, env);
    }

    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        let dynamic = view.as_inner();
        let identity = dynamic.identity();
        push_dynamic_measurement(state, identity, ProposalSize::UNSPECIFIED);
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
            None => match dynamic.with_connected_pending_view_mut(|slot| {
                slot.take().map(|content| {
                    let normalized = normalize_layout_view(content, env);
                    let dimensions = measure_view_dimensions(&normalized, state, env);
                    *slot = Some(normalized);
                    dimensions
                })
            }) {
                Some(Some(dimensions)) => dimensions,
                Some(None) | None => state
                    .dynamic_intrinsic_cache
                    .get(&identity)
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!("hydrolysis Dynamic intrinsic cache miss for connected dynamic node")
                    }),
            },
        };
        state
            .dynamic_intrinsic_cache
            .insert(identity, dimensions.clone());
        pop_dynamic_measurement(state, identity, "after intrinsic measurement");
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
        let proposal_key = proposal_cache_key(identity, proposal);
        push_dynamic_measurement(state, identity, proposal);
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
            None => match dynamic.with_connected_pending_view_mut(|slot| {
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
            }) {
                Some(Some(dimensions)) => dimensions,
                Some(None) | None => {
                    if let Some(dimensions) =
                        state.dynamic_dimensions_cache.get(&proposal_key).cloned()
                    {
                        dimensions
                    } else {
                        let dimensions = state
                            .dynamic_intrinsic_cache
                            .get(&identity)
                            .cloned()
                            .unwrap_or_else(|| {
                                panic!(
                                    "hydrolysis Dynamic intrinsic cache miss for connected dynamic node"
                                )
                            });
                        if proposal != ProposalSize::UNSPECIFIED {
                            state
                                .dynamic_dimensions_cache
                                .insert(proposal_key, dimensions.clone());
                        }
                        dimensions
                    }
                }
            },
        };
        if proposal == ProposalSize::UNSPECIFIED {
            state
                .dynamic_intrinsic_cache
                .insert(identity, dimensions.clone());
        } else {
            state
                .dynamic_dimensions_cache
                .insert(proposal_key, dimensions.clone());
        }
        pop_dynamic_measurement(state, identity, "after dimensions measurement");
        dimensions
    }
}
