//! Per-node retained mounts (water-rs/waterui#1809).
//!
//! Every render node that draws, places children or carries a visual
//! effect owns one Cherenkov layer for its lifetime; a frame touches only
//! what changed. This module grows the machinery in spec order:
//! [`cell`](self::cell) — the node cells, [`Dirty`] marks and [`NodeCore`]
//! embedding; [`placement`](self::placement) — the placement mirror that
//! hit testing and GPU pixel coverage resolve instead of accumulated
//! context transforms; [`program`](self::program) — the per-node record;
//! [`layers`](self::layers) — the retained layer set and the one
//! [`Mount`] a window commits through; [`target`](self::target) — the
//! target seam; [`registry`](self::registry) — the retained registries;
//! `animated` — animated-scalar and morph-progress sampling.

mod animated;
pub mod cell;
pub mod layers;
pub mod placement;
pub mod program;
pub mod registry;
pub mod scopes;
pub mod target;

pub use cell::{Dirty, NodeCell, NodeCore};
pub use layers::{Mount, MountStats};
pub use placement::{HitClasses, HitGate, Placement, PlacementClock, ScopeDelta};
pub use program::{
    MaterialRequest, ProducerContent, ProgramBuilder, SceneContentSource, ScopeKey, ScopeProps,
};
#[cfg(feature = "accessibility")]
pub use registry::Region;
pub use registry::{
    OwnerRegistrations, PaintOrder, PlatformViewRegistration, RegisteredGesture, RetainedEntry,
    RetainedRegistry, SetRegistrationOwner,
};
pub use scopes::RetainedScopes;
pub use target::CherenkovHost;

/// Identity of a producer owner: a counter the renderer hands out through
/// `producer_wake` — never a pointer, so a dropped cell's address cannot
/// resurface as another producer's key.
pub type ProducerKey = u64;
