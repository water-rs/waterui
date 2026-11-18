//! Rendering a view tree through [`AppleBackend`].

use waterui_backend_core::{Environment, Realize, View};

use crate::backend::AppleBackend;
use crate::contract::NativeLeaf;

/// Gives a root environment the state its leaves share.
pub fn install(env: &mut Environment) {
    #[cfg(waterui_feature = "dialog")]
    crate::components::dialog::install(env);
    #[cfg(not(waterui_feature = "dialog"))]
    let _ = env;
}

/// Renders `view` under `env` into the platform view it becomes.
///
/// `mtm` proves the call is on the main thread, where platform views exist.
#[must_use]
pub fn render(view: impl View, env: &Environment, mtm: cocoa_ui::MainThreadMarker) -> NativeLeaf {
    Realize::child(&mut AppleBackend::new(mtm), view, env, ())
}
