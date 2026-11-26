//! Hydrolysis backend.

// A build needs a painter: `gpu` selects the Cherenkov GPU backend, `cpu` the
// Cherenkov CPU rasterizer (the two combine: the offscreen runtime renders on
// the GPU when the host has a usable hardware adapter and on the CPU
// otherwise), and on Android `hwui` the platform `RenderNode` target (which
// composes with `gpu`). A build with none compiles no painter: the theme
// contract (`Style`, `WidgetTheme`, tokens) stays, the renderer, runner and
// platform implementations compile out — the bare check leg exercises exactly
// that shape — and the build script warns on it; the generated launcher
// always enables the resolved painter features.

#[cfg(all(
    feature = "winit",
    not(any(feature = "gpu", feature = "cpu")),
    not(target_os = "android")
))]
compile_error!(
    "hydrolysis: the `winit` runner requires a painter — enable the `gpu` or `cpu` feature"
);

#[cfg(any(
    all(feature = "webview-system", feature = "webview-cef"),
    all(feature = "webview-system", feature = "webview-wpe"),
    all(feature = "webview-cef", feature = "webview-wpe"),
))]
compile_error!(
    "hydrolysis: enable at most one webview engine feature (`webview-system`, `webview-cef`, `webview-wpe`)"
);

/// Whether this build enables `waterui/map` without Hydrolysis's `map-gpu`
/// realization.
#[doc(hidden)]
pub const MAP_ENGINE_MISSING: bool = cfg!(all(waterui_feature = "map", not(feature = "map-gpu")));

/// Whether this build enables `waterui/webview` without a Hydrolysis web
/// engine feature supported on the target platform.
#[doc(hidden)]
pub const WEBVIEW_ENGINE_MISSING: bool = cfg!(all(
    waterui_feature = "webview",
    not(any(
        hydrolysis_webview_engine,
        hydrolysis_android_system_webview
    ))
));

const fn assert_map_capability() {
    #[cfg(all(waterui_feature = "map", not(feature = "map-gpu")))]
    panic!(
        "hydrolysis host enables `waterui/map` without `hydrolysis/map-gpu`; link \
         `waterui-map-gpu` in the application (or enable `hydrolysis/map-gpu`) so \
         Hydrolysis can realize `Map`"
    );
}

const fn assert_webview_capability() {
    #[cfg(all(
        waterui_feature = "webview",
        not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
    ))]
    panic!(
        "hydrolysis host enables `waterui/webview` without a supported web engine \
         feature for this target; enable `webview-cef` or `webview-wpe` (by linking \
         `waterui-browser-cef` or `waterui-browser-wpe`), or target a platform that \
         bridges `webview-system` (`winit` on macOS, `web` on wasm32, or Android)"
    );
}

/// Compile-time capability gate invoked by Hydrolysis host launcher crates.
///
/// Library crates in a Cargo workspace may unify `waterui/map` or
/// `waterui/webview` from unrelated members without enabling a Hydrolysis
/// map or web engine feature, so `hydrolysis` itself cannot emit an
/// unconditional `compile_error!`. Host crates evaluate this function in a
/// `const` item (`const _: () = hydrolysis::assert_host_capabilities();`) so
/// missing engine realizations fail at compile time instead of deferring to
/// runtime `unsupported_map()` / `UNSUPPORTED_WEBVIEW` contract violations.
#[doc(hidden)]
pub const fn assert_host_capabilities() {
    assert_map_capability();
    assert_webview_capability();
}

#[cfg(target_os = "android")]
pub mod android_content;
#[cfg(hydrolysis_runtime)]
mod animation;
#[cfg(hydrolysis_cherenkov)]
mod engine;
// Bare wasm reaches none of the env-driven window/console paths that call
// these helpers.
#[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
#[cfg(hydrolysis_runtime)]
mod env;
#[cfg(hydrolysis_gpu)]
mod gpu_view;
#[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
mod hosted;
// The HWUI render target compiles for tests everywhere and for the Android
// runtime under its feature.
#[cfg(any(test, hydrolysis_hwui))]
pub mod hwui;
#[cfg(hydrolysis_runtime)]
#[cfg(any(test, hydrolysis_localization))]
mod localization;
#[cfg(hydrolysis_runtime)]
mod num_cast;
#[cfg(all(hydrolysis_pipeline_cache, hydrolysis_gpu_painter))]
mod pipeline_cache;
#[cfg(hydrolysis_runtime)]
mod platform;
#[cfg(hydrolysis_runtime)]
mod platform_view;
#[cfg(hydrolysis_gpu_painter)]
mod readback;
#[cfg(hydrolysis_runtime)]
mod renderer;
#[cfg(hydrolysis_runtime)]
mod runner;
#[cfg(all(hydrolysis_runtime, any(test, feature = "testing")))]
pub mod testing;
#[cfg(hydrolysis_runtime)]
mod text;
#[cfg(hydrolysis_runtime)]
mod view_renderer;
#[cfg(hydrolysis_runtime)]
mod widgets;

// Interaction/runtime layer shared with other self-drawn backends.
#[cfg(hydrolysis_runtime)]
pub(crate) use waterui_backend_core::{gesture, scroll, time};

#[cfg(target_os = "android")]
pub use android_content::{
    AndroidContent, AndroidInvalidator, AndroidLifecycle, AndroidMount, AndroidView, jni,
};
#[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
pub use hosted::{HostedContent, HostedObject, HostedOcclusion, HostedView};
use std::time::Duration;

/// The frame interval Hydrolysis budgets for: 120 Hz.
///
/// Budgeted for current high-refresh panels; it stands in for the display's
/// own refresh rate wherever that is not known (headless and offscreen
/// paths, the first frame).
pub const TARGET_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_333);

/// The alpha convention a [`HeadlessSnapshot`]'s pixels read — what the
/// producing surface presented with.
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_cherenkov))]
pub use cherenkov::OutputAlpha;
#[cfg(hydrolysis_gpu_painter)]
pub use cherenkov_gpu::interop::OutputColor;
pub use hydrolysis_style::{Style, draw, theme};
/// The W3C UI Events key vocabulary this backend speaks, re-exported so hosts
/// that synthesize key events use the same version of it.
pub use keyboard_types;
#[cfg(hydrolysis_cpu_painter)]
pub use platform::RasterOffscreen;
#[cfg(hydrolysis_software_surface)]
pub use platform::SoftwareWindowSurface;
#[cfg(all(hydrolysis_winit, hydrolysis_cpu_painter))]
pub use platform::WinitRasterSurface;
#[cfg(hydrolysis_runtime)]
pub use platform::{
    BackEdge, BackNavigation, EdgeBounce, EdgeStretch, ExponentialDecay, FlingDeceleration,
    InputEvent, KeyCode, KeyState, Modifiers, Overscroll, PlatformWindow, PointerButton,
    PointerKind, ReleaseReturn, RubberBand, ScrollPhysics, ScrollPlatform, SplineDeceleration,
    SpringBack, SurfaceError, TextInputPurpose, TextInputState, TouchPhase, TouchScrollConfig,
    WindowKeyboardArea, WindowSafeArea,
};
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::{BrowserWindow, BrowserWindowError};
#[cfg(hydrolysis_cherenkov)]
pub use platform::{
    DEFAULT_HDR_CAPTURE_HEADROOM, HeadlessColorFormat, OffscreenTarget, OffscreenWindow,
};
#[cfg(hydrolysis_gpu_painter)]
pub use platform::{
    DeviceLoss, DeviceLostError, GpuUnavailable, OffscreenGpuContext, OffscreenSceneSurface,
    OffscreenSurface, OpenDeviceError, SurfaceFrame,
};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_cherenkov))]
pub use platform::{OffscreenPainter, PainterUnavailable};
#[cfg(hydrolysis_winit)]
pub use platform::{WinitPainter, WinitPresentation, WinitWindow};
#[cfg(hydrolysis_runtime)]
pub use platform_view::{
    PlacementStretch, PlatformView, PlatformViewPlacement, PlatformViewSink, PlatformViewSource,
};
#[cfg(hydrolysis_gpu_painter)]
pub use readback::{
    ReadbackError, readback_surface_texture_rgba16f, readback_texture_rgba8,
    readback_texture_rgba16f,
};
#[cfg(hydrolysis_cherenkov)]
pub use renderer::FrameError;
#[cfg(all(feature = "frame-profile", hydrolysis_runtime))]
pub use renderer::FrameStageTimes;
#[cfg(all(feature = "frame-profile", hydrolysis_gpu_painter))]
pub use renderer::GpuIdentity;
#[cfg(all(feature = "accessibility", hydrolysis_runtime))]
pub use renderer::accessibility::{AccessibilityActivationPointError, AccessibilityContentTypes};
#[cfg(hydrolysis_runtime)]
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
#[cfg(all(target_os = "android", any(hydrolysis_gpu_painter, hydrolysis_hwui)))]
pub use runner::android;
#[cfg(all(
    hydrolysis_runtime,
    not(any(target_arch = "wasm32", target_os = "android"))
))]
pub use runner::init_tracing;
#[cfg(hydrolysis_run)]
pub use runner::run;
#[cfg(hydrolysis_runtime)]
pub use runner::{FrameCounters, FramePhases, FrameProfile};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_cherenkov))]
pub use runner::{HeadlessHdrSnapshot, HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot};
#[cfg(hydrolysis_cherenkov)]
pub use runner::{SemanticPumpResult, SemanticRuntime};
#[cfg(any(test, hydrolysis_hwui))]
pub use text::HwuiTextEngine;
#[cfg(hydrolysis_cherenkov)]
pub use text::ParleyEngine;
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_cherenkov))]
pub use text::fonts::native_collection;
#[cfg(hydrolysis_runtime)]
pub use text::{DeclaredFonts, FontFamilyResolution};
#[cfg(hydrolysis_runtime)]
pub use view_renderer::HydrolysisOffscreenRenderer;
#[cfg(hydrolysis_macos_system_webview)]
pub use widgets::platform::webview::MacSystemWebViewController;
#[cfg(hydrolysis_macos_system_webview)]
#[doc(hidden)]
pub use widgets::platform::webview::MacSystemWebViewHandle;
#[cfg(waterui_feature = "webview")]
#[doc(hidden)]
pub use widgets::platform::webview::SelectedWebViewController;
#[cfg(hydrolysis_web_system_webview)]
#[doc(hidden)]
pub use widgets::platform::webview::WebIframeHandle;
#[cfg(hydrolysis_web_system_webview)]
pub use widgets::platform::webview::WebSystemWebViewController;
#[cfg(hydrolysis_android_system_webview)]
#[doc(hidden)]
pub use widgets::platform::webview::{AndroidSystemWebViewController, AndroidSystemWebViewHandle};

#[cfg(test)]
mod capability_gate_tests {
    use super::{
        MAP_ENGINE_MISSING, WEBVIEW_ENGINE_MISSING, assert_host_capabilities,
        assert_map_capability, assert_webview_capability,
    };

    #[cfg(all(waterui_feature = "map", not(feature = "map-gpu")))]
    #[test]
    #[should_panic(expected = "hydrolysis/map-gpu")]
    fn map_capability_gate_rejects_map_without_map_gpu() {
        const {
            assert!(MAP_ENGINE_MISSING);
        }
        assert_map_capability();
    }

    #[cfg(not(all(waterui_feature = "map", not(feature = "map-gpu"))))]
    #[test]
    fn map_capability_gate_accepts_build_with_map_engine_or_without_map() {
        const {
            assert!(!MAP_ENGINE_MISSING);
            assert_map_capability();
        }
    }

    #[cfg(all(
        waterui_feature = "webview",
        not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
    ))]
    #[test]
    #[should_panic(expected = "web engine")]
    fn webview_capability_gate_rejects_webview_without_engine() {
        const {
            assert!(WEBVIEW_ENGINE_MISSING);
        }
        assert_webview_capability();
    }

    #[cfg(not(all(
        waterui_feature = "webview",
        not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
    )))]
    #[test]
    fn webview_capability_gate_accepts_build_with_webview_engine_or_without_webview() {
        const {
            assert!(!WEBVIEW_ENGINE_MISSING);
            assert_webview_capability();
        }
    }

    #[cfg(all(
        not(all(waterui_feature = "map", not(feature = "map-gpu"))),
        not(all(
            waterui_feature = "webview",
            not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
        ))
    ))]
    #[test]
    fn host_capability_gate_evaluates_in_const_context_when_satisfied() {
        const _: () = assert_host_capabilities();
    }

    #[cfg(any(
        all(waterui_feature = "map", not(feature = "map-gpu")),
        all(
            waterui_feature = "webview",
            not(any(hydrolysis_webview_engine, hydrolysis_android_system_webview))
        )
    ))]
    #[test]
    #[should_panic(expected = "hydrolysis host enables")]
    fn host_capability_gate_rejects_missing_engine() {
        assert_host_capabilities();
    }
}
