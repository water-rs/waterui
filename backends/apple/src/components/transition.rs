//! The `transition` metadata: `Metadata<TransitionSpec>` — the `.transition(...)`
//! declaration for structural insertion and removal.
//!
//! The handler is transparent to layout and placement. A declaration
//! directly over a conditional — `Native<Dynamic>`, or the
//! `Metadata<Retain>` `watch` puts around one — is adopted by the
//! `Dynamic` host and applies to each content instance it swaps (§5.1);
//! anything else renders to a leaf carrying the declaration as a
//! [`TransitionClaim`] for the structural site that swaps the mount —
//! a `Dynamic` host or a container's id-keyed reconcile. A declaration
//! beneath another modifier stays on the inner leaf and is never
//! claimed, matching the outermost-modifier rule of
//! docs/layout-spec.md §4.5.

use waterui_core::Metadata;
use waterui_core::transition::TransitionSpec;

use crate::dispatch::Dispatcher;
use crate::transition::{SnapshotDemand, TransitionClaim};

/// Installs the `transition` handler on the dispatcher:
/// `Metadata<TransitionSpec>` maps to the content's own leaf carrying
/// the declaration — or, when the content is a conditional, to the
/// adopted `Dynamic` leaf.
///
/// A claim is unconditional — like `Native<()>`, the wrapper can be
/// produced by any view tree since `.transition(...)` lives on `View`
/// itself, so no Cargo feature gates it.
pub fn install(dispatcher: &mut Dispatcher) {
    dispatcher.register_view::<Metadata<TransitionSpec>>(|metadata, ctx| {
        let claim = TransitionClaim::new(metadata.value, ctx.env());
        // `SnapshotDemand` arms every GPU and filtered surface under the
        // declaration for the frozen-frame hand-off — unconditionally,
        // since an asymmetric transition can still resolve to a blur or
        // pixel phase (§2.1).
        let env = ctx.env().extending(SnapshotDemand);
        // Conditional content adopts the declaration for each child it
        // swaps in; without the `dynamic` port nothing conditional renders.
        #[cfg(feature = "dynamic")]
        let content = match crate::components::dynamic::expand_to_dynamic(metadata.content) {
            Ok((dynamic, retained)) => {
                return crate::components::dynamic::render_adopting(
                    dynamic,
                    retained,
                    claim,
                    &ctx.with_env(&env),
                );
            }
            Err(content) => content,
        };
        #[cfg(not(feature = "dynamic"))]
        let content = metadata.content;
        let mut leaf = ctx.with_env(&env).render(content);
        leaf.set_transition(claim);
        leaf
    });
}
