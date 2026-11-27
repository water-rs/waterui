//! Rendering a view tree through [`AppleBackend`].

use waterui_backend_core::{Environment, Realize, View};
#[cfg(waterui_feature = "toggle")]
use waterui_controls::toggle::ToggleStyle;

use crate::backend::AppleBackend;
#[cfg(waterui_feature = "dialog")]
use crate::components::dialog;
use crate::components::sheet;
use crate::contract::NativeLeaf;

/// Gives a root environment the state its leaves share.
pub fn install(env: &mut Environment) {
    #[cfg(waterui_feature = "dialog")]
    dialog::install(env);
    sheet::install(env);
    #[cfg(all(waterui_feature = "toggle", target_os = "macos"))]
    ToggleStyle::install_default(ToggleStyle::Checkbox, env);
    #[cfg(all(waterui_feature = "toggle", target_os = "ios"))]
    ToggleStyle::install_default(ToggleStyle::Switch, env);
}

/// Renders `view` under `env` into the platform view it becomes.
///
/// `mtm` proves the call is on the main thread, where platform views exist.
#[must_use]
pub fn render(view: impl View, env: &Environment, mtm: cocoa_ui::MainThreadMarker) -> NativeLeaf {
    Realize::child(&mut AppleBackend::new(mtm), view, env, ())
}
