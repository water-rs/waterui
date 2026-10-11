//! The TypeScript runtime a preview render installs, when the `ts` feature is
//! on.
//!
//! The support app renders a view it loaded from a dylib into the environment
//! its backend gave it, and a `#[preview]` that returns a `tsx!` view needs a
//! runtime in that environment. The support app has no bundle loader — it is
//! not the application — so `water preview` names the bundle it just built in
//! `WATERUI_TS_BUNDLE`, and every render loads it afresh: the file is read per
//! render, so an edited module shows on the next preview without restarting
//! the support app, and one runtime holds exactly one bundle for exactly one
//! rendered view. The reading of the variable, the path rule and the messages
//! live in `waterui-ts` beside the mount they serve, and `waterui-testing`
//! makes the same call for a test session.

use waterui_core::env::with;
use waterui_core::{AnyView, Environment};
use waterui_internal::ts::{Components, configured_runtime};
use waterui_preview_protocol::PreviewError;

/// `view` with the configured runtime installed for its subtree, or `view`
/// itself when `WATERUI_TS_BUNDLE` is unset.
///
/// The runtime rides in the environment the view's subtree is rendered with
/// (`Metadata<Environment>`, which every backend honours), so it lives exactly
/// as long as the rendered tree and goes with it.
///
/// # Errors
///
/// Returns [`PreviewError::RenderFailed`] when the variable is set but the
/// bundle it names cannot be loaded: the variable is a request to load that
/// bundle, and a render that went on without it would fail at the mount with
/// a message about a missing runtime rather than the missing file.
pub fn with_configured_runtime(env: &Environment, view: AnyView) -> Result<AnyView, PreviewError> {
    match configured_runtime(env, Components) {
        Ok(Some(handle)) => Ok(AnyView::new(with(view, handle))),
        Ok(None) => Ok(view),
        Err(error) => Err(PreviewError::RenderFailed(format!(
            "the preview could not load the TypeScript bundle: {error}"
        ))),
    }
}
