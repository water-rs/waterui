//! Per-node retained mounts (water-rs/waterui#1809).
//!
//! Every render node that draws, places children or carries a visual
//! effect owns one Cherenkov layer for its lifetime; a frame touches only
//! what changed. This module grows the machinery in spec order:
//! [`cell`](self::cell) — the node cells, [`Dirty`] marks and [`NodeCore`]
//! embedding; [`placement`](self::placement) — the placement mirror that
//! hit testing and GPU pixel coverage resolve instead of accumulated
//! context transforms. The layer set (`layers.rs`), recording target
//! (`program.rs`), generic target seam (`target.rs`) and retained
//! registries (`registry.rs`) land in the commits that introduce them.

pub mod cell;
pub mod placement;
pub mod registry;
pub mod scopes;

pub use cell::{Dirty, NodeCell, NodeCore};
pub use placement::{Placement, PlacementClock};
#[cfg(feature = "accessibility")]
pub use registry::Region;
pub use registry::{
    OwnerRegistrations, PaintOrder, PlatformViewRegistration, RegisteredGesture, RetainedEntry,
    RetainedRegistry, SetRegistrationOwner,
};
pub use scopes::RetainedScopes;

/// Identity of a producer owner: a counter the renderer hands out through
/// `producer_wake` — never a pointer, so a dropped cell's address cannot
/// resurface as another producer's key.
pub type ProducerKey = u64;
