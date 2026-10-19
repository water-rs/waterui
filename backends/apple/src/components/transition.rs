//! The `transition` metadata: `Metadata<TransitionSpec>` — the `.transition(...)`
//! declaration for structural insertion and removal.
//!
//! The renderer claims the declaration so the content renders: the leaf is the
//! content's own, the declaration being transparent to layout and placement.
//! The structural contract itself — property transitions projected onto Core
//! Animation, and ghost retention holding the removed member's layout slot
//! until the transition completes (docs/layout-spec.md §4.5) — is
//! water-rs/waterui#1472 and not implemented on this backend yet.

use waterui_core::Metadata;
use waterui_core::transition::TransitionSpec;

use crate::dispatch::Dispatcher;

/// Installs the `transition` handler on the dispatcher: `Metadata<TransitionSpec>`
/// maps to the content's own leaf.
///
/// A claim is unconditional — like `Native<()>`, the wrapper can be produced
/// by any view tree since `.transition(...)` lives on `View` itself, so no
/// Cargo feature gates it.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<TransitionSpec>>(|metadata, ctx| {
        ctx.render(metadata.content)
    });
}
