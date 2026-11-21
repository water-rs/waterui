//! Rendering a view tree through [`AppleBackend`].

use waterui_backend_core::{Environment, Realize, View};

use crate::backend::AppleBackend;
use crate::contract::NativeLeaf;

/// Gives a root environment the state its leaves share.
#[cfg(waterui_feature = "dialog")]
pub fn install(env: &mut Environment) {
    crate::components::dialog::install(env);
}

/// Gives a root environment the state its leaves share.
#[cfg(not(waterui_feature = "dialog"))]
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
