//! The `with_env` metadata: `Metadata<Environment>` wrapped around a
//! child, rendering that child under the metadata's environment.
//!
//! Mirrors the Apple backend's `with_env` port at skeleton scale: the
//! metadata's environment replaces the subtree's — the parent env is not
//! merged back in, because `With::body` already folded it into
//! `metadata.value` through `Environment::extending`. The wrapper owns no
//! platform view of its own: on Apple the host view exists to forward the
//! accent tint and safe-area frame, and neither concept has an Android
//! counterpart in this skeleton, so the handler returns the leaf its child
//! rendered unchanged.

use waterui_backend_core::Environment;
use waterui_core::Metadata;

use crate::dispatch::Dispatcher;

/// Installs the `with_env` handler on the dispatcher: `Metadata<Environment>`
/// renders its content under the metadata's environment.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<Environment>>(|metadata, ctx| {
        ctx.with_env(&metadata.value).render(metadata.content)
    });
}
