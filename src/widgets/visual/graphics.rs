use crate::renderer::{HydroNativeView, HydroState, graphics_dimensions_from_proposal};
use std::rc::Rc;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_graphics::color::Color;
use waterui_graphics::{FilteredView, GpuContentView, Gradient, SceneView, resolve_scene_proposal};
use waterui_shape::{ResolvedMorphShape, ResolvedShape};

impl HydroNativeView for Native<GpuContentView> {
    fn intrinsic(
        _state: &mut HydroState,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        view.as_inner()
            .intrinsic_size()
            .unwrap_or_else(LayoutSize::zero)
    }

    fn dimensions(
        _state: &mut HydroState,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        // Same intrinsic-proposal contract as `SceneView`: a natural size
        // answers it where the container left an axis open, and content that
        // is not fills the proposal.
        graphics_dimensions_from_proposal(resolve_scene_proposal(
            view.as_inner().intrinsic_size(),
            proposal,
        ))
    }
}

impl HydroNativeView for Native<SceneView> {
    fn intrinsic(
        _state: &mut HydroState,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        view.as_inner()
            .intrinsic_size()
            .unwrap_or_else(LayoutSize::zero)
    }

    fn dimensions(
        _state: &mut HydroState,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
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

impl HydroNativeView for Native<FilteredView> {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        crate::renderer::measure_view_intrinsic(&view.as_inner().content, state, env, theme)
    }
}

impl HydroNativeView for Native<Color> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<Gradient> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<ResolvedShape> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl HydroNativeView for Native<ResolvedMorphShape> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::engine::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}
