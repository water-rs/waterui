//! Target-appropriate rendering: a native thread or the owning browser JS thread.
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(not(target_arch = "wasm32"))]
mod native;
pub mod thread;
mod visibility;
#[cfg(target_arch = "wasm32")]
pub use browser::Engine;
#[cfg(not(target_arch = "wasm32"))]
pub use native::Engine;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
pub use visibility::{SurfaceVisibility, WakeGate};

use std::cell::RefCell;
use std::rc::Weak;

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

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
    pub waker: SharedWaker<SurfaceWaker>,
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

/// A host's wake-up callback: transferable on native targets, local on
/// wasm32. Native it may run on any thread — a backend completion or a
/// frame producer's submit wakes the host from where it lands — so it is
/// `Send` to cross threads and `Sync` to be shared across them.
#[cfg(not(target_arch = "wasm32"))]
type Wake = dyn Fn() + Send + Sync;
#[cfg(target_arch = "wasm32")]
type Wake = dyn Fn();

/// The shared owner every waker is handed around in: `Arc` on native
/// targets, where a backend completion can wake the host from any thread;
/// `Rc` on wasm32, where the browser engine and every callback are
/// confined to the owning JS thread. A non-`Send` browser callback is
/// never an atomic, thread-owned entity.
#[cfg(not(target_arch = "wasm32"))]
pub type SharedWaker<T> = Arc<T>;
/// wasm32: local ownership — see the native arm above.
#[cfg(target_arch = "wasm32")]
pub type SharedWaker<T> = Rc<T>;

/// A weak reference to a [`SharedWaker`].
#[cfg(not(target_arch = "wasm32"))]
type WeakWaker<T> = std::sync::Weak<T>;
/// wasm32: a weak reference to an `Rc` — see [`SharedWaker`].
#[cfg(target_arch = "wasm32")]
type WeakWaker<T> = Weak<T>;

/// One surface's host wake-up: the callback the host handed
/// [`Engine::surface`], behind the surface's [`SurfaceVisibility`],
/// coalesced between the surface's renders.
///
/// Every wake that a change on the surface causes goes through it — queued
/// layer and content ops, bound signals, live operands, image replacements
/// the surface draws, and the backend's completions for the surface — as
/// does each engine-scoped event, which [`Wakes`] fans out to every live
/// surface. A hidden surface wakes no host. The flag flips on the UI
/// thread the moment the host announces the visibility, and every wake
/// reads it when it fires; the render loop's own copy, which decides what
/// a frame lists, follows in order with every other message.
pub struct SurfaceWaker {
    /// The host's callback, fixed when the surface was created. Nothing
    /// replaces it, so a wake calls it without loading or borrowing a
    /// slot, and a callback that touches the engine is never reentrant
    /// on the waker.
    wake: Box<Wake>,
    /// Whether the next wake reaches the host: a wake clears it, and the
    /// surface participating in a render sets it again.
    armed: AtomicBool,
    visibility: SurfaceVisibility,
}

impl std::fmt::Debug for SurfaceWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceWaker")
            .field("armed", &self.armed.load(Ordering::Relaxed))
            .field("visibility", &self.visibility.get())
            .finish_non_exhaustive()
    }
}

impl SurfaceWaker {
    /// A visible surface's wake-up through the host's `wake`.
    pub(crate) fn new(wake: Box<Wake>) -> Self {
        Self {
            wake,
            armed: AtomicBool::new(true),
            visibility: SurfaceVisibility::new(),
        }
    }

    /// Wakes the host once if armed, then disarms until the surface next
    /// renders — unless the surface is hidden.
    pub(crate) fn wake(&self) {
        if self.visibility.is_visible() && self.armed.swap(false, Ordering::Relaxed) {
            (self.wake)();
        }
    }

    /// Re-arms the wake after the surface participated in a render.
    pub(crate) fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    /// The visibility the host last announced.
    pub(crate) fn visibility(&self) -> Visibility {
        self.visibility.get()
    }

    /// Stops the surface's wakes.
    pub(crate) fn hide(&self) {
        self.visibility.set(Visibility::Hidden);
    }

    /// Resumes the surface's wakes and asks its host for the frame that
    /// shows it — whether or not the wake is armed, then disarms: the host
    /// may have dropped the frame it requested while every surface it
    /// draws was hidden.
    pub(crate) fn show(&self) {
        self.visibility.set(Visibility::Visible);
        self.armed.store(false, Ordering::Relaxed);
        (self.wake)();
    }

    /// The surface is gone: its wakes — a backend completion landing
    /// after drop, a leaked layer's queued op — stay silent forever.
    pub(crate) fn retire(&self) {
        self.visibility.set(Visibility::Hidden);
    }
}

/// Every live surface's wake, for an event scoped to the engine rather
/// than to one surface — a frame producer's submitted frame. Each
/// surface's own [`SurfaceWaker`] answers it, so the wake is coalesced per
/// surface and a hidden surface wakes nothing.
///
/// The engine republishes the set whenever it creates a surface; a
/// dropped surface's entry no longer upgrades, or upgrades to a retired
/// waker that stays silent.
pub struct Wakes {
    /// The published set, swapped atomically on native — a producer may
    /// submit from any thread — and a local cell on wasm32. A wake
    /// iterates a snapshot, so a callback that creates a surface replaces
    /// the set without disturbing the wake in flight.
    #[cfg(not(target_arch = "wasm32"))]
    surfaces: arc_swap::ArcSwap<Vec<WeakWaker<SurfaceWaker>>>,
    #[cfg(target_arch = "wasm32")]
    surfaces: RefCell<Rc<Vec<WeakWaker<SurfaceWaker>>>>,
}

impl Wakes {
    /// An engine with no surface yet.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn new() -> Self {
        Self {
            surfaces: arc_swap::ArcSwap::default(),
        }
    }

    /// An engine with no surface yet.
    #[cfg(target_arch = "wasm32")]
    pub(crate) fn new() -> Self {
        Self {
            surfaces: RefCell::default(),
        }
    }

    /// Wakes every live surface's host through its own waker.
    pub(crate) fn wake(&self) {
        #[cfg(not(target_arch = "wasm32"))]
        let surfaces = self.surfaces.load_full();
        #[cfg(target_arch = "wasm32")]
        let surfaces = Rc::clone(&self.surfaces.borrow());
        for waker in surfaces.iter().filter_map(WeakWaker::upgrade) {
            waker.wake();
        }
    }

    /// Publishes the wakers of the engine's live surfaces.
    pub(crate) fn publish<B: Backend>(&self, surfaces: &Surfaces<B>) {
        let wakers = surfaces
            .iter()
            .filter(|entry| entry.shared.strong_count() > 0)
            .map(|entry| SharedWaker::downgrade(&entry.waker))
            .collect::<Vec<_>>();
        #[cfg(not(target_arch = "wasm32"))]
        self.surfaces.store(Arc::new(wakers));
        #[cfg(target_arch = "wasm32")]
        self.surfaces.replace(Rc::new(wakers));
    }
}

/// A transferable completion notification for one surface's backend work.
///
/// On native targets this can wake the host from any thread. On wasm32 it
/// retains the engine's single-threaded callback contract. It wakes nothing
/// while the surface is hidden.
#[derive(Debug, Clone)]
pub struct CompletionWaker(SharedWaker<SurfaceWaker>);

impl CompletionWaker {
    /// Wraps a surface's waker. Called on the render loop when the surface
    /// is created.
    pub(crate) fn new(waker: &SharedWaker<SurfaceWaker>) -> Self {
        Self(SharedWaker::clone(waker))
    }

    /// Wakes the surface's host if armed and the surface is visible.
    pub fn wake(&self) {
        self.0.wake();
    }

    /// The surface's visibility as the host announced it, for a source the
    /// backend drives on its own that wakes the host through another
    /// callback: its [`WakeGate`] reads it when a wake fires.
    #[must_use]
    pub fn visibility(&self) -> SurfaceVisibility {
        self.0.visibility.clone()
    }
}
