//! Stable render identity for the retained render tree (water-rs/hydrolysis#205, P5).
//!
//! Distinct from accessibility identity on purpose: a transform/opacity wrapper
//! reports its *child's* `accessibility_identity` (input ancestry looks through
//! transparent wrappers) but owns a separate visual layer, so render identity is
//! never looked through. [`RenderId`] lives exactly as long as the visual node:
//! signal-driven updates of the same node keep it, while a structural
//! replacement — a `Dynamic` rebuild, a collection reconcile removal — lets the
//! replacement's constructor allocate a fresh one.
//!
//! [`PresentationId`] tells apart the placements of the same visual node: its
//! ordinary content placement versus an additional presentation instance (a
//! hosted preview or accessory, a popup surface, a captured replay). Distinct
//! instances of the same node get distinct ids so a preview's engine mount can
//! never collide with — and steal — the original's mount.

use core::sync::atomic::{AtomicU64, Ordering};

/// Lifetime of one visual node in the retained render tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RenderId(u64);

/// One placement of a visual node: its ordinary content placement or an
/// additional presentation instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PresentationId(u64);

/// The engine mount key: which visual node, presented in which placement.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RenderKey {
    /// The visual node the mount belongs to.
    pub(crate) render: RenderId,
    /// The presentation instance the mount belongs to.
    pub(crate) presentation: PresentationId,
}

static NEXT_RENDER_ID: AtomicU64 = AtomicU64::new(1);
static NEXT_PRESENTATION_ID: AtomicU64 = AtomicU64::new(1);

impl RenderId {
    /// Allocate a fresh id for a newly built visual node. Every retained-node
    /// constructor calls this once; updates of the same node reuse it.
    pub(crate) fn next() -> Self {
        Self(NEXT_RENDER_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl PresentationId {
    /// The node's primary placement in the tree that owns it.
    #[allow(dead_code)]
    pub(crate) const ORDINARY: Self = Self(0);

    /// Allocate a fresh id for an additional presentation instance of a node:
    /// a hosted preview or accessory, a popup surface, a captured replay.
    pub(crate) fn next() -> Self {
        Self(NEXT_PRESENTATION_ID.fetch_add(1, Ordering::Relaxed))
    }
}
