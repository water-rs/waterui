//! The TypeScript runtime a session installs, when the `ts` feature is on.
//!
//! A `#[waterui::test]` has no bundle loader of its own, so the session takes
//! the bundle `water test` names in `WATERUI_TS_BUNDLE` and installs the
//! runtime loaded from it into the environment the view is mounted in. The
//! reading of the variable, the path rule and the messages live in
//! `waterui-ts` beside the mount they serve; this is the one call a session
//! makes.

use waterui::ts::{Components, configured_runtime};
use waterui_core::Environment;

/// `env` with the configured runtime installed, or `env` itself when
/// `WATERUI_TS_BUNDLE` is unset.
///
/// # Panics
///
/// Panics when the variable is set but the bundle it names cannot be loaded:
/// the variable is a request to load that bundle, and a test that went on
/// without it would fail later, at the mount, with a message about a missing
/// runtime rather than the missing file.
pub fn install_configured_runtime(env: Environment) -> Environment {
    match configured_runtime(&env, Components) {
        Ok(Some(handle)) => handle.install(&env),
        Ok(None) => env,
        Err(error) => panic!("waterui-testing could not load the TypeScript bundle: {error}"),
    }
}
