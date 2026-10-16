//! The TypeScript runtime a session installs, when the `ts` feature is on.
//!
//! A `#[waterui::test]` has no bundle loader of its own, so the session takes
//! the bundle `water test` names in `WATERUI_TS_BUNDLE` and installs the
//! runtime loaded from it for the view it mounts. The reading of the
//! variable, the path rule and the messages live in `waterui-ts` beside the
//! mount they serve; this is the one call a session makes.
//!
//! The install is scoped to the mounted view rather than to the environment
//! the runtime is constructed with, because a module reads the host's theme
//! — the `ColorScheme` and the colour tokens — out of the environment the
//! runtime was created in, and those are installed by the Hydrolysis runtime
//! and the builder's style, both of which sit above the builder environment.
//! Scoping it to the mounted view is what any host that renders a view
//! outside the application does, for the same reason.

use waterui::ts::{Components, configured_runtime};
use waterui::{Plugin, ViewExt as _};
use waterui_core::{Environment, View};

/// Installs the configured runtime into the environment the mounted view
/// resolves in, or leaves it alone when `WATERUI_TS_BUNDLE` is unset.
struct ConfiguredRuntime;

impl Plugin for ConfiguredRuntime {
    /// # Panics
    ///
    /// Panics when the variable is set but the bundle it names cannot be
    /// loaded: the variable is a request to load that bundle, and a test that
    /// went on without it would fail later, at the mount, with a message
    /// about a missing runtime rather than the missing file.
    fn install(self, env: &mut Environment) {
        match configured_runtime(env, Components) {
            Ok(Some(handle)) => *env = handle.install(env),
            Ok(None) => {}
            Err(error) => panic!("waterui-testing could not load the TypeScript bundle: {error}"),
        }
    }
}

/// `view` with the configured runtime installed for its subtree.
pub fn with_configured_runtime(view: impl View + 'static) -> impl View {
    view.install(ConfiguredRuntime)
}
