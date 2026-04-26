use crate::renderer::{HydroNativeView, HydroState, WidgetRenderContext};
use waterui_core::layout::Size as LayoutSize;
use waterui_core::{Environment, Native};
use waterui_layout::spacer::Spacer;

impl HydroNativeView for Native<()> {
    fn render(_ctx: &mut WidgetRenderContext<'_>, _view: Self, _env: &Environment) {}

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}

impl HydroNativeView for Native<Spacer> {
    fn render(_ctx: &mut WidgetRenderContext<'_>, _view: Self, _env: &Environment) {}

    fn intrinsic(_state: &mut HydroState, _view: &Self, _env: &Environment) -> LayoutSize {
        LayoutSize::zero()
    }
}
