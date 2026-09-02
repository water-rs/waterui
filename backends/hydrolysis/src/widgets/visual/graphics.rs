use crate::renderer::{HydroNativeView, HydroState, graphics_dimensions_from_proposal};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_graphics::color::{Color, ResolvedColor};
use waterui_graphics::view_effect::ViewEffectErased;
use waterui_graphics::{GpuSurface, ResolvedGradient, SceneView, resolve_scene_proposal};
use waterui_shape::{ResolvedMorphShape, ResolvedShape};

impl HydroNativeView for Native<GpuSurface> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<SceneView> {
    fn intrinsic(_state: &mut HydroState, view: &Self, _env: &Environment) -> LayoutSize {
        view.as_inner()
            .intrinsic_size()
            .unwrap_or_else(LayoutSize::zero)
    }

    fn dimensions(
        _state: &mut HydroState,
        view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        // Scene content that is naturally a size answers with it wherever the
        // container left an axis open; content that is not fills the proposal.
        graphics_dimensions_from_proposal(resolve_scene_proposal(
            view.as_inner().intrinsic_size(),
            proposal,
        ))
    }
}

impl HydroNativeView for Native<ViewEffectErased> {
    fn intrinsic(state: &mut HydroState, view: &Self, env: &Environment) -> LayoutSize {
        crate::renderer::measure_view_intrinsic(view.as_inner().content(), state, env)
    }
}

impl HydroNativeView for Native<Color> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<ResolvedColor> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<ResolvedGradient> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<ResolvedShape> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<ResolvedMorphShape> {
    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}
