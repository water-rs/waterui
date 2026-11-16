//! Reports for leaves this build has no realization for: system icons,
//! maps, web views and views that need a GPU device.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
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

/// Reports a leaf that needs a GPU device — `view` names it — reaching a
/// painter without one; `no_gpu` says why this runtime has none.
/// The view is not drawn and measures as zero.
#[cfg(any(not(hydrolysis_gpu), hydrolysis_cpu_painter))]
pub fn unsupported_gpu_device_view(view: &str, no_gpu: &str) {
    contract_violation!(
        "hydrolysis: {view} needs a GPU device, but {no_gpu}; the view is not drawn"
    );
}
