//! Native-leaf measurement: the `HydroNativeView` measure trait, the native-view
//! type list, and the measure-path entry points the layout system uses to size
//! arbitrary sub-views.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::engine::WidgetTheme;
use crate::platform_view::PlatformView;
use std::rc::Rc;

/// The measure half of a native leaf view. Rendering is owned by the retained
/// [`RenderNode`](crate::renderer::tree::RenderNode) tree; this trait only sizes a
/// leaf so the layout system can measure arbitrary sub-views through it.
pub trait HydroNativeView: View + Sized + 'static {
    fn intrinsic(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
    ) -> LayoutSize;
    fn dimensions(
        state: &mut HydroState,
        view: &Self,
        env: &Environment,
        theme: &Rc<dyn WidgetTheme>,
        _proposal: ProposalSize,
    ) -> ViewDimensions {
        ViewDimensions::new(Self::intrinsic(state, view, env, theme))
    }
}

pub fn unsupported_system_icon(icon: &SystemIcon) -> ! {
    panic!(
        "SystemIcon `{}` is unsupported on Hydrolysis because self-drawn backends have no \
         OS-supplied icon catalog; use a packaged WaterUI icon crate",
        icon.name.as_str()
    )
}

impl HydroNativeView for Native<SystemIcon> {
    fn intrinsic(
        _state: &mut HydroState,
        view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn WidgetTheme>,
    ) -> LayoutSize {
        unsupported_system_icon(view.as_inner())
    }
}

/// Reaching `Native<MapConfig>` means `Map::body` found no `Hook<MapConfig>` —
/// no map realization was installed — and this backend ships no map engine of
/// its own.
pub fn unsupported_map() -> ! {
    panic!(
        "Map is unsupported on Hydrolysis because the backend has no map engine; install a \
         map realization such as `waterui_map_gpu::install` before rendering a `Map`"
    )
}

impl HydroNativeView for Native<MapConfig> {
    fn intrinsic(
        _state: &mut HydroState,
        _view: &Self,
        _env: &Environment,
        _theme: &Rc<dyn WidgetTheme>,
    ) -> LayoutSize {
        unsupported_map()
    }
}

/// Reaching `WebView` without a `Hook<WebView>` engine realization means
/// the backend has nothing to draw a page with: a build without
/// `hydrolysis_macos_system_webview` bridges no engine, and the macOS
/// bridge's record has no native-view layer to present the `WKWebView`
/// through.
pub fn unsupported_webview() -> ! {
    panic!(
        "WebView is unsupported on this Hydrolysis build because no web engine is bridged; \
         link a browser engine crate (`waterui-browser-cef`, `waterui-browser-wpe`) or enable \
         the `webview-system` and `winit` features on macOS"
    )
}

/// Reaching a `PlatformView` leaf without a `PlatformViewSink` in the window
/// environment means the runner cannot mount native children at all.
pub fn unsupported_platform_view() -> ! {
    panic!(
        "PlatformView is unsupported on this runner because no PlatformViewSink is installed; \
         a host that embeds native views (the Android runner) inserts one into the window \
         environment at session create"
    )
}

pub fn dimensions_for_native<V: HydroNativeView>(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
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
        $macro!(Native<GpuContentView>);
        $macro!(Native<ExternalFrameView>);
        $macro!(Native<PlatformView>);
        $macro!(Native<SceneView>);
        $macro!(Native<FilteredView>);
        $macro!(Native<Color>);
        $macro!(Native<Gradient>);
        $macro!(Native<ResolvedShape>);
        $macro!(Native<ResolvedMorphShape>);
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

pub fn dimensions_for_known_native_views(
    view: &AnyView,
    proposal: ProposalSize,
    state: &mut HydroState,
    env: &Environment,
    theme: &Rc<dyn WidgetTheme>,
) -> Option<ViewDimensions> {
    macro_rules! try_native_dimensions {
        ($ty:ty) => {
            if let Some(dimensions) =
                dimensions_for_native::<$ty>(view, proposal, state, env, theme)
            {
                return Some(dimensions);
            }
        };
    }
    hydro_native_view_types!(try_native_dimensions);
    None
}
