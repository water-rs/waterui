//! Rendering a view tree through [`AppleBackend`].

#[cfg(waterui_feature = "toggle")]
use waterui::component::toggle::ToggleStyle;
use waterui_backend_core::{Environment, Realize, View};

use crate::backend::AppleBackend;
use crate::contract::NativeLeaf;

/// Gives a root environment the state its leaves share.
#[cfg(any(waterui_feature = "dialog", waterui_feature = "toggle"))]
pub fn install(env: &mut Environment) {
    #[cfg(waterui_feature = "dialog")]
    crate::components::dialog::install(env);
    #[cfg(all(waterui_feature = "toggle", target_os = "macos"))]
    ToggleStyle::install_default(ToggleStyle::Checkbox, env);
    #[cfg(all(waterui_feature = "toggle", target_os = "ios"))]
    ToggleStyle::install_default(ToggleStyle::Switch, env);
}

/// Gives a root environment the state its leaves share.
#[cfg(not(any(waterui_feature = "dialog", waterui_feature = "toggle")))]
pub const fn install(env: &mut Environment) {
    let _ = env;
}

/// Renders `view` under `env` into the platform view it becomes.
///
/// `mtm` proves the call is on the main thread, where platform views exist.
#[must_use]
pub fn render(view: impl View, env: &Environment, mtm: cocoa_ui::MainThreadMarker) -> NativeLeaf {
    Realize::child(&mut AppleBackend::new(mtm), view, env, ())
}
