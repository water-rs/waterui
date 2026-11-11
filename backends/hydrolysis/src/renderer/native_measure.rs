//! Native-leaf measurement: the `HydroNativeView` measure trait, the native-view
//! type list, and the measure-path entry points the layout system uses to size
//! arbitrary sub-views.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform_view::PlatformView;
use crate::theme::WidgetTheme;
use std::rc::Rc;
use waterui_core::contract_violation;

/// The measure half of a native leaf view. Rendering is owned by the retained
/// [`RenderNode`](crate::renderer::tree::RenderNode) tree; this trait only sizes a
/// leaf so the layout system can measure arbitrary sub-views through it.
pub trait HydroNativeView<E: crate::text::TextEngine>: View + Sized + 'static {
    fn intrinsic(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
    ) -> LayoutSize;
    fn dimensions(
        state: &mut HydroState<E>,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
        _proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(Self::intrinsic(state, view, env, theme))
    }
}

/// Reports a `SystemIcon` the backend cannot draw; the icon is not drawn and
/// measures as zero.
pub fn unsupported_system_icon(icon: &SystemIcon) {
    contract_violation!(
        "SystemIcon `{}` is unsupported on Hydrolysis because self-drawn backends have no \
         OS-supplied icon catalog; use a packaged WaterUI icon crate; the icon is not drawn",
        icon.name().as_str()
    );
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<SystemIcon> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn WidgetTheme>,
    ) -> LayoutSize {
        unsupported_system_icon(view.as_inner());
        LayoutSize::zero()
    }
}

/// Reaching `Native<MapConfig>` means `Map::body` found no `Hook<MapConfig>` —
/// no map realization was installed — and this backend ships no map engine of
/// its own. The map is not drawn and measures as zero.
pub fn unsupported_map() {
    contract_violation!(
        "Map is unsupported on Hydrolysis because the backend has no map engine; install a \
         map realization such as `waterui_map_gpu::install` before rendering a `Map`; the map \
         is not drawn"
    );
}

impl<E: crate::text::TextEngine> HydroNativeView<E> for Native<MapConfig> {
    fn intrinsic(
        _state: &mut HydroState<E>,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn WidgetTheme>,
    ) -> LayoutSize {
        unsupported_map();
        LayoutSize::zero()
    }
}

/// Why a `WebView` cannot be drawn on a build without a bridged web engine:
/// reaching `WebView` without a `Hook<WebView>` engine realization leaves the
/// backend nothing to draw a page with. Reported as a contract violation by
/// the tree builder and the leaf's measure.
#[cfg(not(any(hydrolysis_system_webview, hydrolysis_android_system_webview)))]
pub const UNSUPPORTED_WEBVIEW: &str = "WebView is unsupported on this Hydrolysis build because no web engine is bridged; \
     link a browser engine crate (`waterui-browser-cef`, `waterui-browser-wpe`), enable \
     the `webview-system` and `winit` features on macOS, the `webview-system` and \
     `web` features on the web, or the `webview-system` feature on Android";

pub fn dimensions_for_native<V: HydroNativeView<E>, E: crate::text::TextEngine>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> Option<ViewDimensions> {
    view.downcast_ref::<V>()
        .map(|native| V::dimensions(state, native, env, theme, proposal))
}

macro_rules! hydro_native_view_types {
    ($macro:ident) => {
        $macro!(Native<()>);
        $macro!(Native<Spacer>);
        $macro!(Native<TextConfig>);
        $macro!(Native<FixedContainer>);
        $macro!(Native<LazyContainer>);
        $macro!(Native<ScrollView>);
        $macro!(Native<NavigationView>);
        $macro!(Native<NavigationSplitLayout>);
        $macro!(Native<NavigationStack<(), ()>>);
        $macro!(Native<TabsLayout>);
        $macro!(Native<BadgeConfig>);
        $macro!(Native<ListConfig>);
        $macro!(Native<TableConfig>);
        $macro!(Native<ButtonConfig>);
        $macro!(Native<ResolvedMenu>);
        $macro!(Native<ToggleConfig>);
        $macro!(Native<SliderConfig>);
        $macro!(Native<StepperConfig>);
        $macro!(Native<ProgressConfig>);
        $macro!(Native<ColorPickerConfig>);
        $macro!(Native<DatePickerConfig>);
        $macro!(Native<ResolvedTextFieldConfig>);
        $macro!(Native<SecureFieldConfig>);
        $macro!(Native<PickerConfig>);
        $macro!(Native<Dynamic>);
        $macro!(Native<SystemIcon>);
        #[cfg(hydrolysis_gpu)]
        $macro!(Native<GpuContentView>);
        #[cfg(hydrolysis_gpu)]
        $macro!(Native<ExternalFrameView>);
        $macro!(Native<PlatformView>);
        #[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
        $macro!(Native<crate::HostedView>);
        $macro!(Native<SceneView>);
        $macro!(Native<CanvasView>);
        $macro!(Native<FilteredView>);
        $macro!(Native<Color>);
        $macro!(Native<Gradient>);
        $macro!(Native<ResolvedShape>);
        $macro!(Native<MapConfig>);
        $macro!(WebView);
    };
}

pub fn is_hydro_native_view(view: &AnyView) -> bool {
    macro_rules! check_native_view {
        ($ty:ty) => {
            if view.downcast_ref::<$ty>().is_some() {
                return true;
            }
        };
    }
    hydro_native_view_types!(check_native_view);
    false
}

pub fn dimensions_for_known_native_views<E: crate::text::TextEngine>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState<E>,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> Option<ViewDimensions> {
    macro_rules! try_native_dimensions {
        ($ty:ty) => {
            if let Some(dimensions) =
                dimensions_for_native::<$ty, E>(view, proposal, state, env, theme)
            {
                return Some(dimensions);
            }
        };
    }
    hydro_native_view_types!(try_native_dimensions);
    None
}
