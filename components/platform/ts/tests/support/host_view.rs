//! The environment a host that mounts a `tsx!` view outside an application
//! gives the view's subtree: the runtime loaded from the bundle
//! `WATERUI_TS_BUNDLE` names, scoped to the view rather than the session.
//!
//! `waterui-testing` installs it into the mounted view's environment for a
//! test session; any other host does the same through
//! [`with_configured_runtime`], so the module's `Mount` finds a runtime in
//! the tree above it and reads the host's theme from the environment the
//! runtime was created in.

use waterui::ts::{BundleError, Components, configured_runtime};
use waterui_core::env::with;
use waterui_core::{AnyView, Environment};

/// `view` with the configured runtime installed for its subtree, or `view`
/// itself when `WATERUI_TS_BUNDLE` is unset.
///
/// # Errors
///
/// Returns [`BundleError`] when the variable is set but the bundle it names
/// cannot be loaded: the variable is a request to load that bundle, and a
/// render that went on without it would fail at the mount with a message
/// about a missing runtime rather than the missing file.
pub fn with_configured_runtime(
    environment: &Environment,
    view: AnyView,
) -> Result<AnyView, BundleError> {
    match configured_runtime(environment, Components)? {
        Some(handle) => Ok(AnyView::new(with(view, handle))),
        None => Ok(view),
    }
}
