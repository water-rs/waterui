//! Target-appropriate rendering: a native thread or the owning browser JS thread.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
pub mod thread;
mod wake;
#[cfg(target_arch = "wasm32")]
pub use browser::Engine;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Engine;
pub use wake::{CompletionWaker, FrameScope, SurfaceWaker, SurfaceWakes};

use std::cell::RefCell;
use std::rc::Weak;
use std::sync::Arc;

use crate::backend::{Backend, Visibility};
use crate::frame::{FrameTime, Next};

use cherenkov_record::{ChangeSet, Shared, SurfaceId};

/// A live surface's engine-side entry: its shared UI-thread queue state
/// (weak — the `Surface` handle owns it), its host waker, and the cell the
/// frame's per-surface [`Next`] is published into.
pub struct SurfaceEntry<B: Backend> {
    /// The shared pending-changes state.
    pub shared: Weak<RefCell<Shared<B>>>,
    /// The surface's host wake-up; the queue reads visibility through it.
    pub waker: Arc<SurfaceWaker>,
    /// The cell `Surface::next_frame` reads.
    pub next_frame: Weak<RefCell<Next>>,
}

/// The engine's surfaces, by their shared UI-thread state.
type Surfaces<B> = Vec<SurfaceEntry<B>>;

/// Whether the engine has a surface and every one is hidden: a render
/// would have nothing it may draw.
fn all_hidden<B: Backend>(surfaces: &Surfaces<B>) -> bool {
    let mut live = surfaces
        .iter()
        .filter(|entry| entry.shared.strong_count() > 0)
        .peekable();
    live.peek().is_some() && live.all(|entry| entry.waker.visibility() == Visibility::Hidden)
}

/// Drains every visible surface's queued changes, sampled at `time`, into
/// `commits`, and forgets dropped surfaces. A hidden surface has sent its
/// changes as it made them; it is neither drained nor sampled until it is
/// visible again.
///
/// Every visible surface participates in the frame, changed or not, so
/// each one's own wake is re-armed here: a queued change wakes its host
/// at most once until the surface next renders.
fn drain_visible<B: Backend>(
    surfaces: &mut Surfaces<B>,
    time: FrameTime,
    commits: &mut Vec<(SurfaceId, ChangeSet<B>)>,
) {
    surfaces.retain(|entry| {
        let Some(shared) = entry.shared.upgrade() else {
            return false;
        };
        if entry.waker.visibility() == Visibility::Visible {
            entry.waker.arm();
            let mut shared_mut = shared.borrow_mut();
            if let Some(changes) = shared_mut.take_changes(time.0) {
                commits.push((shared_mut.id, changes));
            }
        }
        true
    });
}

/// Publishes the frame's per-surface deadlines: every live surface gets
/// the [`Next`] the frame produced for it — its own animation/backend
/// deadline — and `Idle` when the frame carried nothing for it, so a
/// hidden surface never reads another surface's demand. Lookup is the
/// frame's keyed map, so publication is linear in the surface count.
fn publish_next<B: Backend>(
    surfaces: &Surfaces<B>,
    frame_next: &rustc_hash::FxHashMap<SurfaceId, Next>,
) {
    for entry in surfaces {
        let (Some(shared), Some(next_frame)) = (entry.shared.upgrade(), entry.next_frame.upgrade())
        else {
            continue;
        };
        let next = frame_next
            .get(&shared.borrow().id)
            .map_or(Next::Idle, Clone::clone);
        next_frame.replace(next);
    }
}
