use crate::renderer::{HydroNativeView, HydroState, graphics_dimensions_from_proposal};
use std::rc::Rc;
use waterui_core::layout::Size as LayoutSize;
use waterui_core::layout::{ProposalSize, ViewDimensions};
use waterui_core::{Environment, Native};
use waterui_graphics::color::Color;
#[cfg(hydrolysis_gpu)]
use waterui_graphics::{ExternalFrameView, GpuContentView};
use waterui_graphics::{FilteredView, Gradient, SceneView, resolve_scene_proposal};
use waterui_shape::{ResolvedMorphShape, ResolvedShape};

#[cfg(hydrolysis_gpu)]
impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<GpuContentView> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        view.as_inner().measure(ProposalSize::UNSPECIFIED).size
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        // The view's own measure asks the live content: a source whose
        // size resolves after mount reports it here, where the
        // `intrinsic_size` snapshot would keep answering what the content
        // claimed at construction (water-rs/waterui#1958).
        view.as_inner().measure(proposal)
    }
}

#[cfg(hydrolysis_gpu)]
impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ExternalFrameView> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        view.as_inner().measure(ProposalSize::UNSPECIFIED).size
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        // The view's own measure honours its intrinsic size where the
        // container left an axis open and fills the proposal otherwise.
        view.as_inner().measure(proposal)
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<SceneView> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        view.as_inner()
            .intrinsic_size()
            .unwrap_or_else(LayoutSize::zero)
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
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

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<FilteredView> {
    fn intrinsic(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        crate::renderer::measure_view_intrinsic(&view.as_inner().content, state, env, theme)
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<Color> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<Gradient> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ResolvedShape> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<ResolvedMorphShape> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
    ) -> LayoutSize {
        LayoutSize::zero()
    }

    fn dimensions(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn crate::theme::WidgetTheme>,
        proposal: ProposalSize,
    ) -> ViewDimensions {
        graphics_dimensions_from_proposal(proposal)
    }
}
