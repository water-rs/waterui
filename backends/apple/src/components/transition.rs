//! The `transition` metadata: `Metadata<TransitionSpec>` — the `.transition(...)`
//! declaration for structural insertion and removal.
//!
//! The handler is transparent to layout and placement. A declaration
//! directly over a conditional — a `Dynamic`, or the `Metadata<Retain>`
//! `watch` puts around one — is adopted by the `Dynamic` host through an
//! [`AdoptionOffer`] and applies to each content instance it swaps (§5.1);
//! anything else renders to a leaf carrying the declaration as a
//! [`TransitionClaim`] for the structural site that swaps the mount —
//! a `Dynamic` host or a container's id-keyed reconcile. A declaration
//! beneath another modifier stays on the inner leaf and is never
//! claimed, matching the outermost-modifier rule of
//! docs/layout-spec.md §4.5.

use waterui_core::Metadata;
use waterui_structural_transition::TransitionSpec;

use crate::contract::{NativeLeaf, RenderContext};
use crate::transition::adoption::AdoptionOffer;
use crate::transition::{SnapshotDemand, TransitionClaim};

/// The `transition` realization:
/// `Metadata<TransitionSpec>` maps to the content's own leaf carrying
/// the declaration — or, when the content is a conditional, to the
/// adopted `Dynamic` leaf.
///
/// A claim is unconditional — like `Native<()>`, the wrapper can be
/// produced by any view tree since `.transition(...)` lives on `View`
/// itself, so no Cargo feature gates it.
pub fn realize_transition_spec_metadata(
    metadata: Metadata<TransitionSpec>,
    ctx: &RenderContext<'_>,
) -> NativeLeaf {
    let claim = TransitionClaim::new(metadata.value, ctx.env());
    // `SnapshotDemand` arms every GPU and filtered surface under the
    // declaration for the frozen-frame hand-off — unconditionally,
    // since an asymmetric transition can still resolve to a blur or
    // pixel phase (§2.1). The adoption offer goes to the content's first
    // payload: a conditional there adopts the declaration for each child
    // it swaps in.
    let offer = AdoptionOffer::new(claim.clone());
    let env = ctx.env().extending(SnapshotDemand).extending(offer.clone());
    let mut leaf = ctx.with_env(&env).render(metadata.content);
    if !offer.adopted() {
        leaf.set_transition(claim);
    }
    leaf
}
