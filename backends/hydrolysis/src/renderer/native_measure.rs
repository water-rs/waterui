//! Native leaves: the native-view type list layout normalization stops at,
//! and the reports for leaves this build has no realization for (system
//! icons, maps, web views, views that need a GPU device).

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use crate::platform_view::PlatformView;
use waterui_core::contract_violation;

/// Reports a `SystemIcon` the backend cannot draw; the icon is not drawn and
/// measures as zero.
pub fn unsupported_system_icon(icon: &SystemIcon) {
    contract_violation!(
        "SystemIcon `{}` is unsupported on Hydrolysis because self-drawn backends have no \
         OS-supplied icon catalog; use a packaged WaterUI icon crate; the icon is not drawn",
        icon.name().as_str()
    );
}

/// Reaching `Native<MapConfig>` means `Map::body` found no `Hook<MapConfig>` —
/// no map realization was installed — and this backend ships no map engine of
/// its own. The map is not drawn and measures as zero.
#[cfg(waterui_feature = "map")]
pub fn unsupported_map() {
    contract_violation!(
        "Map is unsupported on Hydrolysis because the backend has no map engine; install a \
         map realization such as `waterui_map_gpu::install` before rendering a `Map`; the map \
         is not drawn"
    );
}

/// Why a `WebView` cannot be drawn on a build without a bridged web engine:
/// reaching `WebView` without a `Hook<WebView>` engine realization leaves the
/// backend nothing to draw a page with. Reported as a contract violation by
/// the tree builder and the leaf's measure.
#[cfg(waterui_feature = "webview")]
#[cfg(not(any(hydrolysis_system_webview, hydrolysis_android_system_webview)))]
pub const UNSUPPORTED_WEBVIEW: &str = "WebView is unsupported on this Hydrolysis build because no web engine is bridged; \
     link a browser engine crate (`waterui-browser-cef`, `waterui-browser-wpe`), enable \
     the `webview-system` and `winit` features on macOS, the `webview-system` and \
     `web` features on the web, or the `webview-system` feature on Android";

/// Reports a leaf that needs a GPU device — `view`, as
/// [`gpu_device_view`](waterui_graphics::backend::gpu_device_view) names it —
/// reaching a painter without one; `no_gpu` says why this runtime has none.
/// The view is not drawn and measures as zero.
#[cfg(any(not(hydrolysis_gpu), hydrolysis_cpu_painter))]
pub fn unsupported_gpu_device_view(view: &str, no_gpu: &str) {
    contract_violation!(
        "hydrolysis: {view} needs a GPU device, but {no_gpu}; the view is not drawn"
    );
}

/// Refuses `view` when it needs a GPU device, in a build without the GPU
/// painter: such a build compiles no realization for these leaves, so measure
/// and node build stop at them here instead of expanding their `body`.
/// Returns whether `view` was refused; the caller then measures it as zero
/// and builds it as an empty node.
#[cfg(not(hydrolysis_gpu))]
pub fn refuse_gpu_device_view(view: &AnyView) -> bool {
    let Some(name) = waterui_graphics::backend::gpu_device_view(view) else {
        return false;
    };
    unsupported_gpu_device_view(
        name,
        "this Hydrolysis build has no GPU painter: it was built without the `gpu` feature",
    );
    true
}

macro_rules! hydro_native_view_types {
    ($macro:ident) => {
        $macro!(Native<()>);
        $macro!(Native<Spacer>);
        $macro!(Native<TextConfig>);
        $macro!(Native<FixedContainer>);
        $macro!(Native<LazyContainer>);
        $macro!(Native<ScrollView>);
        #[cfg(feature = "navigation")]
        $macro!(Native<NavigationView>);
        #[cfg(feature = "navigation")]
        $macro!(Native<NavigationSplitLayout>);
        #[cfg(feature = "navigation")]
        $macro!(Native<NavigationStack<(), ()>>);
        #[cfg(feature = "navigation")]
        $macro!(Native<TabsLayout>);
        $macro!(Native<BadgeConfig>);
        #[cfg(feature = "list")]
        $macro!(Native<ListConfig>);
        #[cfg(feature = "table")]
        $macro!(Native<TableConfig>);
        $macro!(Native<ButtonConfig>);
        #[cfg(feature = "menu")]
        $macro!(Native<ResolvedMenu>);
        #[cfg(feature = "toggle")]
        $macro!(Native<ToggleConfig>);
        #[cfg(feature = "slider")]
        $macro!(Native<SliderConfig>);
        #[cfg(feature = "stepper")]
        $macro!(Native<StepperConfig>);
        $macro!(Native<ProgressConfig>);
        #[cfg(feature = "color-picker")]
        $macro!(Native<ColorPickerConfig>);
        #[cfg(feature = "date-picker")]
        $macro!(Native<DatePickerConfig>);
        #[cfg(feature = "text-field")]
        $macro!(Native<ResolvedTextFieldConfig>);
        #[cfg(feature = "secure-field")]
        $macro!(Native<SecureFieldConfig>);
        #[cfg(feature = "picker")]
        $macro!(Native<PickerConfig>);
        $macro!(Native<Dynamic>);
        $macro!(Native<SystemIcon>);
        #[cfg(hydrolysis_gpu)]
        $macro!(Native<GpuView>);
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
        #[cfg(waterui_feature = "map")]
        $macro!(Native<MapConfig>);
        #[cfg(waterui_feature = "webview")]
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
    // A leaf that needs a GPU device is a leaf here too, so layout stops at
    // it and measure and node build refuse it by name.
    #[cfg(not(hydrolysis_gpu))]
    if waterui_graphics::backend::gpu_device_view(view).is_some() {
        return true;
    }
    false
}
