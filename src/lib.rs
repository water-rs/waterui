//! Hydrolysis backend.
//!
//! `HydrolysisExt` provides `.hydrolysis()` to wrap any cloneable view into
//! a `GpuSurface` rendered by hydrolysis.

mod engine;
mod env;
mod gpu_view;
mod localization;
#[cfg(hydrolysis_pipeline_cache)]
mod pipeline_cache;
mod platform;
mod readback;
mod renderer;
mod runner;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod theme;
mod view_renderer;
mod widgets;

// Interaction/runtime layer shared with other self-drawn backends.
pub(crate) use waterui_backend_core::{animation, gesture, scroll, time};

pub use engine::{Brush, DrawContext, IconOnlyButtonLabel, WidgetTheme};
use std::time::Duration;
use waterui_core::Environment;

/// The frame interval Hydrolysis budgets for: 120 Hz, because current devices
/// have high-refresh panels. It stands in for the display's own refresh rate
/// wherever that is not known (headless and offscreen paths, the first frame).
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
pub use gpu_view::{HydrolysisExt, HydrolysisGpuView};
/// The W3C UI Events key vocabulary this backend speaks, re-exported so hosts
/// that synthesize key events use the same version of it.
pub use keyboard_types;
#[cfg(all(target_arch = "wasm32", feature = "web"))]
pub use platform::BrowserWindow;
#[cfg(hydrolysis_winit)]
pub use platform::WinitWindow;
pub use platform::{
    GpuSurfaceWindow, InputEvent, KeyCode, KeyState, Modifiers, OffscreenGpuContext,
    OffscreenSurface, OffscreenWindow, PlatformWindow, PointerButton, PointerKind, SurfaceError,
    SurfaceFrame, SurfaceProvider, TextInputPurpose, TextInputState, TouchPhase, WindowSafeArea,
};
#[cfg(feature = "frame-profile")]
pub use renderer::{FrameStageTimes, GpuIdentity};
pub use renderer::{HydroState, HydrolysisRenderTarget, HydrolysisRenderer, RenderContext};
#[cfg(target_os = "android")]
pub use runner::android;
#[cfg(not(target_os = "android"))]
pub use runner::run;
pub use runner::{FrameCounters, FramePhases, FrameProfile, SemanticPumpResult, SemanticRuntime};
#[cfg(not(target_arch = "wasm32"))]
pub use runner::{HeadlessPumpResult, HeadlessRuntime, HeadlessSnapshot};
pub use view_renderer::HydrolysisViewRenderer;
#[cfg(hydrolysis_macos_system_webview)]
pub use widgets::platform::webview::MacSystemWebViewController;
