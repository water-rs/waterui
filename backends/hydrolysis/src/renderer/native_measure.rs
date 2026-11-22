//! Reports for leaves this build has no realization for: maps, web views
//! and views that need a GPU device.

#[cfg(any(
    all(waterui_feature = "map", not(feature = "map-gpu")),
    all(not(hydrolysis_gpu), waterui_feature = "gpu"),
    all(hydrolysis_gpu, hydrolysis_cpu_painter)
))]
use waterui_core::contract_violation;

/// Reports a `Map` reaching a build without the GPU map (`map-gpu`): this
/// backend bridges no platform map. The map is not drawn and measures as zero.
#[cfg(all(waterui_feature = "map", not(feature = "map-gpu")))]
pub fn unsupported_map() {
    contract_violation!(
        "Map is unsupported on this Hydrolysis build because it has no map engine; link \
         `waterui-map-gpu` so the backend draws a `Map` with it; the map is not drawn"
    );
}

/// Why a `WebView` cannot be drawn on a build without a selected web engine.
/// Reported as a contract violation by the tree builder and the leaf's measure.
#[cfg(waterui_feature = "webview")]
#[cfg(not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview)))]
pub const UNSUPPORTED_WEBVIEW: &str = "WebView is unsupported on this Hydrolysis build because no web engine is bridged; \
     enable the `webview-cef` or `webview-wpe` feature, the `webview-system` and `winit` \
     features on macOS, the `webview-system` and `web` features on the web, or the \
     `webview-system` feature on Android";

/// Reports a leaf that needs a GPU device — `view` names it — reaching a
/// painter without one; `no_gpu` says why this runtime has none.
/// The view is not drawn and measures as zero.
#[cfg(any(
    all(not(hydrolysis_gpu), waterui_feature = "gpu"),
    all(hydrolysis_gpu, hydrolysis_cpu_painter)
))]
pub fn unsupported_gpu_device_view(view: &str, no_gpu: &str) {
    contract_violation!(
        "hydrolysis: {view} needs a GPU device, but {no_gpu}; the view is not drawn"
    );
}
