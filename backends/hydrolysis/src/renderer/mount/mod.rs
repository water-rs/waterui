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
//! `animated` — animated-scalar and morph-progress sampling; `stretch` —
//! the overscroll stretch filter a scroll node's inner layer draws with.

mod animated;
pub mod backdrop;
pub mod cell;
#[cfg(any(test, hydrolysis_hwui))]
pub mod hwui;
pub mod layers;
pub mod placement;
pub mod program;
pub mod registry;
pub mod scopes;
mod stretch;
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
#[cfg(hydrolysis_gpu_painter)]
pub use target::CherenkovHost;
#[cfg(hydrolysis_gpu_painter)]
pub use target::MaterialTerms;

/// Identity of a producer owner: a counter the renderer hands out through
/// `producer_wake` — never a pointer, so a dropped cell's address cannot
/// resurface as another producer's key.
pub type ProducerKey = u64;

impl<E: crate::text::TextEngine> crate::renderer::HydrolysisRenderer<E> {
    /// Commits the mounted tree through `mount` against `host` at `scale`,
    /// then rolls its pass statistics and the target's resource
    /// registrations into the frame counters.
    pub(crate) fn commit_mount<T: target::LayerTarget<E>>(
        &mut self,
        mount: &mut Mount<T, E>,
        host: &T::Host,
        scale: f64,
    ) {
        let roots = self.mount_roots();
        let host_wake = self.host_redraw_handle.clone();
        let window_material = self.core.window_material;
        let core = &mut self.core;
        let mut wakes =
            |cell: &std::rc::Rc<NodeCell<E>>| core.producer_wake(cell, host_wake.clone());
        mount.commit(
            host,
            self.window_display_transform,
            scale,
            &roots,
            &mut wakes,
            window_material.as_ref(),
        );
        self.core.clear_commit_marks();
        let taken = mount.take_stats();
        self.last_mount_stats = layers::census(&roots, taken);
        self.state.counters.recorded_view_contents += taken.installs + taken.live_records;
        self.state.counters.live_recordings += taken.live_records;
        self.state.counters.layer_creations += taken.created;
        self.state.counters.layer_removals += taken.removed;
        let (fonts, images) = T::resources(host).take_registration_stats();
        self.state.counters.font_registrations += fonts;
        self.state.counters.image_registrations += images;
    }
}
