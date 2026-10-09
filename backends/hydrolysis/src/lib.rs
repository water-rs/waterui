//! Hydrolysis backend.

#[cfg(not(any(feature = "cherenkov", hydrolysis_hwui)))]
compile_error!(
    "hydrolysis has no render target: enable `cherenkov` (the default) or, on Android, `hwui`"
);
#[cfg(all(feature = "testing", not(feature = "cherenkov")))]
compile_error!("the `testing` harness drives the Cherenkov renderer: enable `cherenkov`");

mod engine;
#[cfg(any(hydrolysis_hwui, test))]
pub mod hwui;
// Bare wasm reaches none of the env-driven window/console paths that call
// these helpers.
#[cfg_attr(all(target_arch = "wasm32", not(feature = "web")), allow(dead_code))]
#[cfg(feature = "cherenkov")]
mod env;
#[cfg(feature = "cherenkov")]
mod gpu_view;
#[cfg(feature = "cherenkov")]
mod localization;
#[cfg(feature = "cherenkov")]
mod num_cast;
#[cfg(all(hydrolysis_pipeline_cache, feature = "cherenkov"))]
mod pipeline_cache;
#[cfg(feature = "cherenkov")]
mod platform;
#[cfg(feature = "cherenkov")]
mod platform_view;
#[cfg(feature = "cherenkov")]
mod readback;
#[cfg(feature = "cherenkov")]
mod renderer;
#[cfg(feature = "cherenkov")]
mod runner;
#[cfg(all(any(test, feature = "testing"), feature = "cherenkov"))]
pub mod testing;
#[cfg(any(feature = "cherenkov", hydrolysis_hwui))]
mod text;
#[cfg(feature = "cherenkov")]
pub mod theme;
#[cfg(feature = "cherenkov")]
mod view_renderer;
#[cfg(feature = "cherenkov")]
mod widgets;

// Interaction/runtime layer shared with other self-drawn backends.
#[cfg(feature = "cherenkov")]
pub(crate) use waterui_backend_core::{animation, gesture, scroll, time};

pub use engine::{IconOnlyButtonLabel, WidgetTheme};
use std::time::Duration;
use waterui_core::Environment;

/// The frame interval Hydrolysis budgets for: 120 Hz.
///
/// Budgeted for current high-refresh panels; it stands in for the display's
/// own refresh rate wherever that is not known (headless and offscreen
/// paths, the first frame).
pub const TARGET_FRAME_INTERVAL: Duration = Duration::from_nanos(8_333_333);

/// A presentation style for the rendered Hydrolysis runtime.
///
/// A `Style` is the runtime's widget theme plus the package's token set:
/// the runtime assembles the environment's theme tokens with framework
/// defaults beneath [`Self::install_tokens`] beneath the application's own
/// environment (`crate::theme::install_theme_tokens`), and finally owns the
/// style itself and hands `&dyn WidgetTheme` to the layout and encode
/// contexts.
///
/// The semantic runtime (`SemanticRuntime`) takes no `Style`: widget
/// structure, roles, labels, values, states and actions never depend on one.
/// A theme read during view build or patch is a compile error by
/// construction — build and patch contexts cannot reach a theme.
pub trait Style: WidgetTheme + 'static {
    /// Installs the style package's colour, font and other environment
    /// tokens. The environment already carries the application's entries
    /// layered over the framework defaults, so an installed value can be
    /// read here (e.g. the application's colour scheme), and the
    /// application's own entries still win in the assembled environment.
    fn install_tokens(&self, env: &mut Environment);
}
/// The W3C UI Events key vocabulary this backend speaks, re-exported so hosts
/// that synthesize key events use the same version of it.
pub use keyboard_types;

#[cfg(all(feature = "cherenkov", target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(all(feature = "cherenkov", hydrolysis_winit))]
pub use platform::WinitWindow;
#[cfg(all(feature = "cherenkov", feature = "accessibility"))]
pub use renderer::accessibility::AccessibilityActivationPointError;
#[cfg(all(feature = "cherenkov", feature = "frame-profile"))]
pub use renderer::{FrameStageTimes, GpuIdentity};
#[cfg(all(feature = "cherenkov", target_os = "android"))]
pub use runner::android;
#[cfg(all(feature = "cherenkov", hydrolysis_run))]
pub use runner::run;
#[cfg(all(feature = "cherenkov", not(target_arch = "wasm32")))]
pub use runner::{HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot};
#[cfg(all(feature = "cherenkov", hydrolysis_macos_system_webview))]
pub use widgets::platform::webview::MacSystemWebViewController;
#[cfg(feature = "cherenkov")]
pub use {
    platform::{
        BackEdge, BackNavigation, FlingDeceleration, GpuSurfaceWindow, InputEvent, KeyCode,
        KeyState, Modifiers, OffscreenGpuContext, OffscreenSceneSurface, OffscreenSurface,
        OffscreenWindow, PlatformWindow, PointerButton, PointerKind, SurfaceError, SurfaceFrame,
        SurfaceProvider, TextInputPurpose, TextInputState, TouchPhase, TouchScrollConfig,
        WindowKeyboardArea, WindowSafeArea,
    },
    platform_view::{PlatformView, PlatformViewPlacement, PlatformViewSink},
    readback::{ReadbackError, readback_texture_rgba8},
    renderer::{HydroState, HydrolysisRenderTarget, HydrolysisRenderer, RenderContext},
    runner::{FrameCounters, FramePhases, FrameProfile, SemanticPumpResult, SemanticRuntime},
    text::FontFamilyResolution,
    view_renderer::HydrolysisViewRenderer,
};
