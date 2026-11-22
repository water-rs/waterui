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

/// Stops filter-chain expansion at native leaves and metadata boundaries.
#[cfg(any(feature = "applied_filter", feature = "view_effect"))]
pub(crate) fn is_native_boundary(view: &waterui_backend_core::AnyView) -> bool {
    let name = view.name();
    name.starts_with("waterui_core::components::native::Native<")
        || name.starts_with("waterui_core::components::metadata::Metadata<")
}

/// Renders `view` under `env` into the platform view it becomes.
///
/// `mtm` proves the call is on the main thread, where platform views exist.
#[must_use]
pub fn render(view: impl View, env: &Environment, mtm: cocoa_ui::MainThreadMarker) -> NativeLeaf {
    Realize::child(&mut AppleBackend::new(mtm), view, env, ())
}
