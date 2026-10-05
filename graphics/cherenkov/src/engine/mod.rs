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

#[cfg(not(target_arch = "wasm32"))]
use arc_swap::ArcSwapOption;

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

/// The callback is transferable on native targets and local on wasm32.
/// Native it may run on any thread while the engine holds it, so it is
/// shared: `Send` to cross threads, `Sync` to be shared across them.
#[cfg(not(target_arch = "wasm32"))]
type Wake = dyn Fn() + Send + Sync;
#[cfg(target_arch = "wasm32")]
type Wake = dyn Fn();

/// The shared owner every waker and callback is handed around in: `Arc`
/// on native targets, where a backend completion can wake the host from
/// any thread; `Rc` on wasm32, where the browser engine and every
/// callback are confined to the owning JS thread. A non-`Send` browser
/// callback is never an atomic, thread-owned entity.
#[cfg(not(target_arch = "wasm32"))]
pub type SharedWaker<T> = Arc<T>;
/// wasm32: local ownership — see the native arm above.
#[cfg(target_arch = "wasm32")]
pub type SharedWaker<T> = Rc<T>;

/// A sized holder for the unsized callback — arc-swap's `RefCnt` needs
/// a sized `Arc` payload.
#[cfg(not(target_arch = "wasm32"))]
struct WakeBox(Arc<Wake>);

/// The host wake-up, coalesced between engine renders.
/// Native callbacks may run on any thread; the callback slot is swapped
/// atomically, so installing or replacing a callback never waits on a
/// wake in flight.
pub struct Waker {
    /// The callback slot: atomically swapped on native, a local cell on
    /// wasm32 — a browser callback is `Fn()`, so it lives in no atomic
    /// container there.
    #[cfg(not(target_arch = "wasm32"))]
    callback: ArcSwapOption<WakeBox>,
    #[cfg(target_arch = "wasm32")]
    callback: RefCell<Option<Rc<Wake>>>,
    armed: AtomicBool,
}

impl std::fmt::Debug for Waker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Waker")
            .field(
                "armed",
                &self.armed.load(std::sync::atomic::Ordering::Relaxed),
            )
            .finish_non_exhaustive()
    }
}

impl Waker {
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn new() -> Self {
        Self {
            callback: ArcSwapOption::empty(),
            armed: AtomicBool::new(true),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(super) fn new() -> Self {
        Self {
            callback: RefCell::new(None),
            armed: AtomicBool::new(true),
        }
    }

    /// Calls the callback once if armed, then disarms until the next
    /// [`Engine::render`] re-arms.
    fn wake(&self) {
        if self.armed.swap(false, Ordering::Relaxed) {
            self.call();
        }
    }

    /// Calls the callback whether or not it is armed, then disarms: a
    /// surface becoming visible must reach a host that dropped the frame
    /// it requested while every surface it draws was hidden.
    fn wake_now(&self) {
        self.armed.store(false, Ordering::Relaxed);
        self.call();
    }

    /// Loaded or cloned out of the slot first — the callback may itself
    /// touch the engine, so a reentrant `set_waker` replaces an
    /// unborrowed slot and the call in flight stays valid.
    #[cfg(not(target_arch = "wasm32"))]
    fn call(&self) {
        let Some(callback) = self.callback.load_full() else {
            return;
        };
        (callback.0)();
    }

    /// wasm32: clones the callback out of its local cell before
    /// invoking — same reentrancy contract as the native arm.
    #[cfg(target_arch = "wasm32")]
    fn call(&self) {
        let callback = self.callback.borrow().clone();
        let Some(callback) = callback else {
            return;
        };
        (callback)();
    }

    /// Installs or replaces the callback.
    #[cfg(not(target_arch = "wasm32"))]
    fn set(&self, f: SharedWaker<Wake>) {
        self.callback.store(Some(Arc::new(WakeBox(f))));
    }

    /// wasm32: writes the callback into its local cell — see `call`.
    #[cfg(target_arch = "wasm32")]
    fn set(&self, f: SharedWaker<Wake>) {
        *self.callback.borrow_mut() = Some(f);
    }

    /// Whether a callback is installed.
    #[cfg(not(target_arch = "wasm32"))]
    fn has_callback(&self) -> bool {
        self.callback.load().is_some()
    }

    /// wasm32: reads the local cell — see `call`.
    #[cfg(target_arch = "wasm32")]
    fn has_callback(&self) -> bool {
        self.callback.borrow().is_some()
    }

    pub(super) fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }
}

/// One surface's host wake-up: the engine's [`Waker`] behind the surface's
/// [`SurfaceVisibility`].
///
/// Every wake that a change on the surface causes goes through it — queued
/// layer and content ops, bound signals, live operands, image replacements
/// the surface draws, and the backend's completions for the surface — so a
/// hidden surface wakes no host. The flag flips on the UI thread the moment
/// the host announces the visibility, and every wake reads it when it
/// fires; the render loop's own copy, which decides what a frame lists,
/// follows in order with every other message.
#[derive(Debug)]
pub struct SurfaceWaker {
    /// The engine's aggregate wake — the target every wake takes until
    /// the surface installs its own callback through
    /// [`Surface::set_waker`](crate::Surface::set_waker).
    engine: SharedWaker<Waker>,
    /// The surface's own wake, coalesced and re-armed independently of
    /// the engine's: a change on this surface wakes only this surface's
    /// host.
    own: Waker,
    visibility: SurfaceVisibility,
    /// Whether the surface's pending queue holds unrendered work — the
    /// queue's own bookkeeping, written where `pending` is mutated, so
    /// [`Surface::set_waker`](crate::Surface::set_waker) reads it without
    /// borrowing the shared state (which can legitimately be borrowed
    /// when the callback itself queues work).
    unrendered: AtomicBool,
}

impl SurfaceWaker {
    /// A visible surface's wake-up through `engine`.
    pub(crate) fn new(engine: SharedWaker<Waker>) -> Self {
        Self {
            engine,
            own: Waker::new(),
            visibility: SurfaceVisibility::new(),
            unrendered: AtomicBool::new(false),
        }
    }

    /// Records whether the pending queue is nonempty — called at each
    /// write to it, never inferred from a borrow.
    pub(crate) fn note_pending(&self, nonempty: bool) {
        self.unrendered.store(nonempty, Ordering::Relaxed);
    }

    /// Whether the pending queue holds unrendered work.
    pub(crate) fn has_pending(&self) -> bool {
        self.unrendered.load(Ordering::Relaxed)
    }

    /// Installs the surface's own wake-up. Queued changes, backend
    /// completions and reveals route through it instead of the engine's.
    pub(crate) fn set_callback(&self, f: SharedWaker<Wake>) {
        self.own.set(f);
    }

    /// The wake a change on the surface reaches: its own callback when
    /// one is installed, else the engine's aggregate wake.
    fn target(&self) -> &Waker {
        if self.own.has_callback() {
            &self.own
        } else {
            &self.engine
        }
    }

    /// Wakes the host, coalesced between the surface's renders, unless
    /// the surface is hidden.
    pub(crate) fn wake(&self) {
        if self.visibility.is_visible() {
            self.target().wake();
        }
    }

    /// Re-arms the surface's own wake after the surface participated in a
    /// render; the engine's wake is re-armed once per render by the
    /// engine itself.
    pub(crate) fn arm(&self) {
        self.own.arm();
    }

    /// The visibility the host last announced.
    pub(crate) fn visibility(&self) -> Visibility {
        self.visibility.get()
    }

    /// Stops the surface's wakes.
    pub(crate) fn hide(&self) {
        self.visibility.set(Visibility::Hidden);
    }

    /// Resumes the surface's wakes and asks its own host for the frame
    /// that shows it.
    pub(crate) fn show(&self) {
        self.visibility.set(Visibility::Visible);
        self.target().wake_now();
    }

    /// The surface is gone: its wakes — a backend completion landing
    /// after drop, a leaked layer's queued op — stay silent forever.
    pub(crate) fn retire(&self) {
        self.visibility.set(Visibility::Hidden);
        self.note_pending(false);
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

    /// Fires the host's wake callback if armed and the surface is visible.
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
