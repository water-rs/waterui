//! The `transition` metadata: `Metadata<TransitionSpec>` — the `.transition(...)`
//! declaration for structural insertion and removal.
//!
//! The handler is transparent to layout and placement: it renders the
//! content and stores the declaration on the leaf it returns, as a
//! [`TransitionClaim`]. The structural site that swaps the mount — a
//! `Dynamic` host or a container's id-keyed reconcile — claims it at the
//! root of the mounted subtree, drives the effect through
//! [`crate::transition`], and holds the removed member's slot as a ghost
//! until the clock ends (docs/layout-spec.md §4.5). A declaration beneath
//! another modifier stays on the inner leaf and is never claimed.

use waterui_core::Metadata;
use waterui_core::transition::{ReduceMotion, TransitionSpec};

use crate::dispatch::Dispatcher;
use crate::transition::TransitionClaim;

/// Installs the `transition` handler on the dispatcher: `Metadata<TransitionSpec>`
/// maps to the content's own leaf carrying the declaration.
///
/// A claim is unconditional — like `Native<()>`, the wrapper can be produced
/// by any view tree since `.transition(...)` lives on `View` itself, so no
/// Cargo feature gates it.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<TransitionSpec>>(|metadata, ctx| {
        let reduce_motion = ctx.env().get::<ReduceMotion>().is_some_and(|flag| flag.0);
        let mut leaf = ctx.render(metadata.content);
        leaf.set_transition(TransitionClaim {
            spec: metadata.value,
            reduce_motion,
        });
        leaf
    });
}
