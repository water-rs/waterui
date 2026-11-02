//! Hydrolysis backend.

// An Android build needs one painter: `hwui` selects the platform `RenderNode`
// target, `gpu` the Cherenkov GPU attachment (the two may combine). A build
// with neither compiles no painter: the theme contract (`Style`,
// `WidgetTheme`, tokens) stays, the renderer, runner and platform
// implementations compile out — the bare check leg exercises exactly that
// shape — and the build script warns on it; the generated launcher always
// enables the resolved painter's feature.

#[cfg(hydrolysis_runtime)]
mod animation;
#[cfg(hydrolysis_gpu_painter)]
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
pub mod theme;
#[cfg(hydrolysis_gpu_painter)]
mod view_renderer;
#[cfg(hydrolysis_runtime)]
mod widgets;

// Interaction/runtime layer shared with other self-drawn backends.
#[cfg(hydrolysis_runtime)]
pub(crate) use waterui_backend_core::{gesture, scroll, time};

#[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
pub use hosted::{HostedContent, HostedObject, HostedOcclusion, HostedView};
use std::time::Duration;
use waterui_core::Environment;

/// The frame interval Hydrolysis budgets for: 120 Hz.
///
/// Budgeted for current high-refresh panels; it stands in for the display's
/// own refresh rate wherever that is not known (headless and offscreen
/// paths, the first frame).
pub const TARGET_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_333);

/// A presentation style for the rendered Hydrolysis runtime: the type a style
/// package implements, and what `run(app, env, style)` accepts.
///
/// A `Style` is the runtime's widget theme plus the package's token set:
/// the runtime assembles the environment's theme tokens with framework
/// defaults beneath [`Self::install_tokens`] beneath the application's own
/// environment (`crate::theme::install_theme_tokens`), and finally owns the
/// style itself and hands `&dyn` [`WidgetTheme`](theme::WidgetTheme) to the
/// layout and encode contexts. The [`theme`] module documents the whole
/// contract a style package targets.
///
/// The semantic runtime (`SemanticRuntime`) takes no `Style`: widget
/// structure, roles, labels, values, states and actions never depend on one.
/// A theme read during view build or patch is a compile error by
/// construction — build and patch contexts cannot reach a theme.
pub trait Style: theme::WidgetTheme + 'static {
    /// Installs the style package's colour, font and other environment
    /// tokens. The environment already carries the application's entries
    /// layered over the framework defaults, so an installed value can be
    /// read here (e.g. the application's colour scheme), and the
    /// application's own entries still win in the assembled environment.
    fn install_tokens(&self, env: &mut Environment);
}
/// The alpha convention a [`HeadlessSnapshot`]'s pixels read — what the
/// producing surface presented with.
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
pub use cherenkov_gpu::interop::OutputAlpha;
/// The W3C UI Events key vocabulary this backend speaks, re-exported so hosts
/// that synthesize key events use the same version of it.
pub use keyboard_types;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(hydrolysis_winit)]
pub use platform::WinitWindow;
#[cfg(hydrolysis_runtime)]
pub use platform::{
    BackEdge, BackNavigation, EdgeBounce, EdgeStretch, ExponentialDecay, FlingDeceleration,
    InputEvent, KeyCode, KeyState, Modifiers, Overscroll, PlatformWindow, PointerButton,
    PointerKind, ReleaseReturn, RubberBand, ScrollPhysics, ScrollPlatform, SplineDeceleration,
    SpringBack, SurfaceError, TextInputPurpose, TextInputState, TouchPhase, TouchScrollConfig,
    WindowKeyboardArea, WindowSafeArea,
};
#[cfg(hydrolysis_gpu_painter)]
pub use platform::{
    DeviceLoss, OffscreenGpuContext, OffscreenSceneSurface, OffscreenSurface, OffscreenWindow,
    SurfaceFrame,
};
#[cfg(hydrolysis_runtime)]
pub use platform_view::{
    PlatformView, PlatformViewPlacement, PlatformViewSink, PlatformViewSource,
};
#[cfg(hydrolysis_gpu_painter)]
pub use readback::{ReadbackError, readback_texture_rgba8};
#[cfg(hydrolysis_gpu_painter)]
pub use renderer::HydrolysisRenderTarget;
#[cfg(all(feature = "accessibility", hydrolysis_runtime))]
pub use renderer::accessibility::{AccessibilityActivationPointError, AccessibilityContentTypes};
#[cfg(all(feature = "frame-profile", hydrolysis_gpu_painter))]
pub use renderer::{FrameStageTimes, GpuIdentity};
#[cfg(hydrolysis_runtime)]
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
#[cfg(all(target_os = "android", any(hydrolysis_gpu_painter, hydrolysis_hwui)))]
pub use runner::android;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use runner::init_tracing;
#[cfg(hydrolysis_run)]
pub use runner::run;
#[cfg(hydrolysis_runtime)]
pub use runner::{FrameCounters, FramePhases, FrameProfile};
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
pub use runner::{HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot};
#[cfg(hydrolysis_gpu_painter)]
pub use runner::{SemanticPumpResult, SemanticRuntime};
#[cfg(any(test, hydrolysis_hwui))]
pub use text::HwuiTextEngine;
#[cfg(hydrolysis_gpu_painter)]
pub use text::ParleyEngine;
#[cfg(all(not(target_arch = "wasm32"), hydrolysis_gpu_painter))]
pub use text::fonts::native_collection;
#[cfg(hydrolysis_runtime)]
pub use text::{DeclaredFonts, FontFamilyResolution};
#[cfg(hydrolysis_gpu_painter)]
pub use view_renderer::HydrolysisViewRenderer;
#[cfg(hydrolysis_macos_system_webview)]
pub use widgets::platform::webview::MacSystemWebViewController;
#[cfg(hydrolysis_web_system_webview)]
pub use widgets::platform::webview::WebSystemWebViewController;
