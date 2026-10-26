//! Hydrolysis backend.

// An Android build needs one painter: `hwui` selects the platform `RenderNode`
// target, `gpu` the Cherenkov GPU attachment (the two may combine). Neither
// means no painter and no GPU deps for one to draw through.
#[cfg(all(
    android_platform,
    not(hydrolysis_hwui),
    not(feature = "gpu")
))]
compile_error!(
    "hydrolysis on Android needs a painter: enable the `gpu` or `hwui` feature"
);

mod animation;
#[cfg(not(hydrolysis_hwui))]
mod engine;
// Bare wasm reaches none of the env-driven window/console paths that call
// these helpers.
#[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
mod env;
#[cfg(hydrolysis_gpu)]
mod gpu_view;
#[cfg(all(hydrolysis_hosted, hydrolysis_gpu))]
mod hosted;
// The HWUI render target compiles for tests everywhere and for the Android
// runtime under its feature.
#[cfg(any(test, hydrolysis_hwui))]
pub mod hwui;
mod localization;
mod num_cast;
#[cfg(all(hydrolysis_pipeline_cache, not(hydrolysis_hwui)))]
mod pipeline_cache;
mod platform;
mod platform_view;
#[cfg(not(hydrolysis_hwui))]
mod readback;
mod renderer;
mod runner;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
mod text;
pub mod theme;
#[cfg(not(hydrolysis_hwui))]
mod view_renderer;
mod widgets;

// Interaction/runtime layer shared with other self-drawn backends.
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
#[cfg(all(not(target_arch = "wasm32"), not(hydrolysis_hwui)))]
pub use cherenkov_gpu::interop::OutputAlpha;
/// The W3C UI Events key vocabulary this backend speaks, re-exported so hosts
/// that synthesize key events use the same version of it.
pub use keyboard_types;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(hydrolysis_winit)]
pub use platform::WinitWindow;
#[cfg(not(hydrolysis_hwui))]
pub use platform::{
    DeviceLoss, OffscreenGpuContext, OffscreenSceneSurface, OffscreenSurface, OffscreenWindow,
    SurfaceFrame,
};
pub use platform::{
    BackEdge, BackNavigation, EdgeBounce, EdgeStretch, ExponentialDecay, FlingDeceleration,
    InputEvent, KeyCode, KeyState, Modifiers, Overscroll, PlatformWindow, PointerButton,
    PointerKind, ReleaseReturn, RubberBand, ScrollPhysics, ScrollPlatform, SplineDeceleration,
    SpringBack, SurfaceError, TextInputPurpose, TextInputState, TouchPhase, TouchScrollConfig,
    WindowKeyboardArea, WindowSafeArea,
};
pub use platform_view::{
    PlatformView, PlatformViewPlacement, PlatformViewSink, PlatformViewSource,
};
#[cfg(not(hydrolysis_hwui))]
pub use readback::{ReadbackError, readback_texture_rgba8};
#[cfg(feature = "accessibility")]
pub use renderer::accessibility::{AccessibilityActivationPointError, AccessibilityContentTypes};
#[cfg(all(feature = "frame-profile", not(hydrolysis_hwui)))]
pub use renderer::{FrameStageTimes, GpuIdentity};
#[cfg(not(hydrolysis_hwui))]
pub use renderer::HydrolysisRenderTarget;
pub use renderer::{HydroState, HydrolysisRenderer, RenderContext};
#[cfg(target_os = "android")]
pub use runner::android;
#[cfg(not(any(target_arch = "wasm32", target_os = "android")))]
pub use runner::init_tracing;
#[cfg(hydrolysis_run)]
pub use runner::run;
pub use runner::{FrameCounters, FramePhases, FrameProfile};
#[cfg(not(hydrolysis_hwui))]
pub use runner::{SemanticPumpResult, SemanticRuntime};
#[cfg(all(not(target_arch = "wasm32"), not(hydrolysis_hwui)))]
pub use runner::{HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot};
#[cfg(any(test, hydrolysis_hwui))]
pub use text::HwuiTextEngine;
#[cfg(all(not(target_arch = "wasm32"), not(hydrolysis_hwui)))]
pub use text::fonts::native_collection;
pub use text::{DeclaredFonts, FontFamilyResolution};
#[cfg(not(hydrolysis_hwui))]
pub use text::ParleyEngine;
#[cfg(not(hydrolysis_hwui))]
pub use view_renderer::HydrolysisViewRenderer;
#[cfg(hydrolysis_macos_system_webview)]
pub use widgets::platform::webview::MacSystemWebViewController;
#[cfg(hydrolysis_web_system_webview)]
pub use widgets::platform::webview::WebSystemWebViewController;
