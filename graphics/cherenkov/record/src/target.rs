//! The render target a [`Shared`](crate::Shared) queue and a
//! [`SurfaceTree`](crate::SurfaceTree) describe, and the capabilities a
//! target may lack.
//!
//! `cherenkov-record` runs no engine: a target owns its layer queue on its
//! own thread and decides how queued changes reach it through
//! [`Target::Queue`]. The capability traits ([`ProjectiveLayers`],
//! [`BackdropSampling`], [`GpuInstalls`]) bound the edit API so a target
//! that cannot consume a property cannot be asked for it.

use crate::ops::ChangeSet;

/// A layer tree's render target.
///
/// The queueing side — [`Shared`](crate::Shared), [`LayerContent`],
/// [`LayerEdit`](crate::LayerEdit), [`Transaction`](crate::Transaction) and
/// the [`Op`](crate::ops::Op)s and [`ChangeSet`] they produce — is generic
/// over `T`, so a target chooses its own queue endpoint and install
/// payload. [`Layer`] and [`SurfaceTree`](crate::SurfaceTree) are
/// target-neutral.
///
/// [`Layer`]: crate::Layer
/// [`LayerContent`]: crate::LayerContent
pub trait Target: Sized + 'static {
    /// The queue endpoint [`Shared`](crate::Shared) notifies when changes
    /// arrive: static dispatch, no boxing.
    type Queue: Queue<Self>;
    /// The payload [`LayerContent::install`] seals in [`Install`] for the
    /// change set's [`Op::Install`](crate::ops::Op::Install) — an engine
    /// backend's install closure. The sealing is what keeps a target
    /// without [`GpuInstalls`] from producing one.
    ///
    /// [`LayerContent::install`]: crate::LayerContent::install
    /// [`Install`]: crate::Install
    /// [`GpuInstalls`]: crate::GpuInstalls
    type Install;
}

/// How [`Shared`](crate::Shared) hands queued changes to its target.
///
/// `Shared` never holds a borrowed hook: it drains internally, then calls
/// exactly one of these methods. A queue that answers `true` to
/// [`drains_inline`](Queue::drains_inline) — a hidden surface, for a
/// consumer that applies changes without a frame — is handed each drained
/// [`ChangeSet`] through [`apply`](Queue::apply); a visible queue is told to
/// schedule a drain through [`wake`](Queue::wake).
pub trait Queue<T: Target> {
    /// Whether queued changes drain inline through
    /// [`apply`](Queue::apply) rather than scheduling a drain through
    /// [`wake`](Queue::wake).
    fn drains_inline(&self) -> bool;
    /// Applies a drained change set. Called only with `Some` — an empty
    /// drain sends nothing.
    fn apply(&self, changes: ChangeSet<T>);
    /// Wakes the consumer: queued changes wait for its next drain.
    fn wake(&self);
}

/// The target samples projective layer transforms: `LayerEdit::projection`,
/// `tilt`, `depth` and `clear_projection` exist only when a target
/// implements this.
///
/// [`LayerEdit`]: crate::LayerEdit
pub trait ProjectiveLayers: Target {}

/// The target samples backdrop groups: `LayerEdit::backdrop` and
/// `clear_backdrop` exist only when a target implements this.
///
/// [`LayerEdit`]: crate::LayerEdit
pub trait BackdropSampling: Target {}

/// The target installs engine-produced content into layers:
/// [`LayerContent::install`](crate::LayerContent::install) exists only when
/// a target implements this.
pub trait GpuInstalls: Target {}
