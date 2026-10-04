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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
pub use visibility::{SurfaceVisibility, WakeGate};

use std::cell::RefCell;
use std::rc::Weak;

use crate::backend::{Backend, Visibility};
use crate::frame::FrameTime;
use crate::message::{ChangeSet, SurfaceId};
use crate::surface::Shared;

/// The engine's surfaces, by their shared UI-thread state.
type Surfaces<B> = Vec<Weak<RefCell<Shared<B>>>>;

/// Whether the engine has a surface and every one is hidden: a render
/// would have nothing it may draw.
fn all_hidden<B: Backend>(surfaces: &Surfaces<B>) -> bool {
    let mut live = surfaces.iter().filter_map(Weak::upgrade).peekable();
    live.peek().is_some() && live.all(|shared| shared.borrow().visibility() == Visibility::Hidden)
}

/// Drains every visible surface's queued changes, sampled at `time`, into
/// `commits`, and forgets dropped surfaces. A hidden surface has sent its
/// changes as it made them; it is neither drained nor sampled until it is
/// visible again.
fn drain_visible<B: Backend>(
    surfaces: &mut Surfaces<B>,
    time: FrameTime,
    commits: &mut Vec<(SurfaceId, ChangeSet<B>)>,
) {
    surfaces.retain(|weak| {
        let Some(shared) = weak.upgrade() else {
            return false;
        };
        let mut shared_mut = shared.borrow_mut();
        if shared_mut.visibility() == Visibility::Visible
            && let Some(changes) = shared_mut.take_changes(time.0)
        {
            commits.push((shared_mut.id, changes));
        }
        true
    });
}

/// The callback is transferable on native targets and local on wasm32.
/// Native it may run on any thread while the engine holds it, so it is
/// shared: `Send` to cross threads, `Sync` to be shared across them.
#[cfg(not(target_arch = "wasm32"))]
type Wake = dyn Fn() + Send + Sync;
#[cfg(target_arch = "wasm32")]
type Wake = dyn Fn();

/// The host wake-up, coalesced between engine renders.
/// Native callbacks may run on any thread; the callback slot is synchronized.
pub struct Waker {
    callback: Mutex<Option<Arc<Wake>>>,
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
    pub(super) fn new() -> Self {
        Self {
            callback: Mutex::new(None),
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

    fn call(&self) {
        // Cloned under the lock, called outside it: the callback may
        // itself touch the engine — a `set_waker` it makes replaces the
        // slot — and a panicking callback must not lose the installed
        // one.
        let callback = self.callback.lock().expect("waker poisoned").clone();
        let Some(callback) = callback else { return };
        callback();
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
    engine: Arc<Waker>,
    visibility: SurfaceVisibility,
}

impl SurfaceWaker {
    /// A visible surface's wake-up through `engine`.
    pub(crate) fn new(engine: Arc<Waker>) -> Self {
        Self {
            engine,
            visibility: SurfaceVisibility::new(),
        }
    }

    /// Wakes the host, coalesced between renders, unless the surface is
    /// hidden.
    pub(crate) fn wake(&self) {
        if self.visibility.is_visible() {
            self.engine.wake();
        }
    }

    /// The visibility the host last announced.
    pub(crate) fn visibility(&self) -> Visibility {
        self.visibility.get()
    }

    /// Stops the surface's wakes.
    pub(crate) fn hide(&self) {
        self.visibility.set(Visibility::Hidden);
    }

    /// Resumes the surface's wakes and asks the host for the frame that
    /// shows it.
    pub(crate) fn show(&self) {
        self.visibility.set(Visibility::Visible);
        self.engine.wake_now();
    }
}

/// A transferable completion notification for one surface's backend work.
///
/// On native targets this can wake the host from any thread. On wasm32 it
/// retains the engine's single-threaded callback contract. It wakes nothing
/// while the surface is hidden.
#[derive(Debug, Clone)]
pub struct CompletionWaker(Arc<SurfaceWaker>);

impl CompletionWaker {
    /// Wraps a surface's waker. Called on the render loop when the surface
    /// is created.
    pub(crate) fn new(waker: &Arc<SurfaceWaker>) -> Self {
        Self(Arc::clone(waker))
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
