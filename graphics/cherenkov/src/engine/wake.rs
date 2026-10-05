//! The engine's one wake mechanism: each surface's host wake-up, and the
//! fan-out of an event a backend source causes to the surfaces it draws
//! into.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arc_swap::ArcSwap;

use crate::backend::Visibility;

/// A host's wake-up callback. It may run on any thread a wake starts on —
/// the UI thread, the render thread, a backend completion, a producer's or
/// a filter parameter's thread — so it is `Send` to cross threads and
/// `Sync` to be shared across them, on every target.
type Wake = dyn Fn() + Send + Sync;

/// One surface's host wake-up: the callback the host handed
/// [`Engine::surface`](crate::Engine::surface), behind the visibility the
/// host announced for the surface, coalesced between the surface's renders.
///
/// Every wake on the surface's behalf goes through it — queued layer and
/// content ops, bound signals, live operands, image replacements and
/// producer frames the surface draws, the backend's completions for the
/// surface, and the redraw requests of the producers and filters drawn on
/// it ([`SurfaceWakes`]). A hidden surface wakes no host: the visibility
/// flips on the UI thread the moment the host announces it, and every wake
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
    /// Whether the host last announced the surface visible.
    visible: AtomicBool,
}

impl std::fmt::Debug for SurfaceWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceWaker")
            .field("armed", &self.armed.load(Ordering::Relaxed))
            .field("visibility", &self.visibility())
            .finish_non_exhaustive()
    }
}

impl SurfaceWaker {
    /// A visible surface's wake-up through the host's `wake`.
    pub(crate) fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            wake: Box::new(wake),
            armed: AtomicBool::new(true),
            visible: AtomicBool::new(true),
        }
    }

    /// Wakes the host once if armed, then disarms until the surface next
    /// renders — unless the surface is hidden.
    pub(crate) fn wake(&self) {
        if self.visible.load(Ordering::Acquire) && self.armed.swap(false, Ordering::Relaxed) {
            (self.wake)();
        }
    }

    /// Re-arms the wake after the surface participated in a render.
    pub(crate) fn arm(&self) {
        self.armed.store(true, Ordering::Relaxed);
    }

    /// Disarms the wake until the surface next renders: the host is
    /// building the frame whose render drains whatever a wake would ask
    /// for.
    pub(crate) fn disarm(&self) {
        self.armed.store(false, Ordering::Relaxed);
    }

    /// The visibility the host last announced.
    pub(crate) fn visibility(&self) -> Visibility {
        if self.visible.load(Ordering::Acquire) {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    }

    /// Stops the surface's wakes.
    pub(crate) fn hide(&self) {
        self.visible.store(false, Ordering::Release);
    }

    /// Resumes the surface's wakes and asks its host for the frame that
    /// shows it — whether or not the wake is armed, then disarms: the host
    /// may have dropped the frame it requested while every surface it
    /// draws was hidden.
    pub(crate) fn show(&self) {
        self.visible.store(true, Ordering::Release);
        self.armed.store(false, Ordering::Relaxed);
        (self.wake)();
    }

    /// The surface is gone: its wakes — a backend completion landing
    /// after drop, a leaked layer's queued op, a producer or filter whose
    /// wakes still list it — stay silent forever.
    pub(crate) fn retire(&self) {
        self.hide();
    }
}

/// A transferable handle to one surface's [`SurfaceWaker`], handed to the
/// backend when the surface is created
/// ([`Renderer::create_surface`](crate::Renderer::create_surface)).
///
/// It wakes the host from any thread, coalesced with every other wake on
/// the surface's behalf, and wakes nothing while the surface is hidden or
/// after it is dropped. Two handles are equal when they wake the same
/// surface.
#[derive(Debug, Clone)]
pub struct CompletionWaker(Arc<SurfaceWaker>);

impl CompletionWaker {
    /// Wraps a surface's waker. Called on the render loop when the surface
    /// is created.
    pub(crate) fn new(waker: &Arc<SurfaceWaker>) -> Self {
        Self(Arc::clone(waker))
    }

    /// Wakes the surface's host if armed and the surface is visible.
    pub fn wake(&self) {
        self.0.wake();
    }
}

impl PartialEq for CompletionWaker {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for CompletionWaker {}

/// The wakes of the surfaces a backend source draws into.
///
/// The source is one the backend drives on its own — a rendered GPU
/// producer, a filter. A redraw request it makes off the render path
/// reaches each of those surfaces' hosts through the surface's own
/// [`SurfaceWaker`]: coalesced per surface until it next renders, silent
/// while it is hidden and after it is dropped.
///
/// The render loop sets the surfaces from its frames, so the membership
/// can lag a frame behind the content; the surface that installs the
/// source was woken by the install itself. Each surface's visibility is
/// read when the wake fires, so the visibility never lags. A source starts
/// with no surface and wakes nothing.
#[derive(Debug)]
pub struct SurfaceWakes(ArcSwap<Vec<CompletionWaker>>);

impl Default for SurfaceWakes {
    fn default() -> Self {
        Self(ArcSwap::from_pointee(Vec::new()))
    }
}

impl SurfaceWakes {
    /// Wakes the host of every surface the source draws into, each through
    /// its own wake.
    pub fn wake(&self) {
        for waker in self.0.load().iter() {
            waker.wake();
        }
    }

    /// Replaces the surfaces the source draws into. Setting the surfaces it
    /// already has allocates nothing.
    pub fn set(&self, surfaces: &[CompletionWaker]) {
        if self.0.load().as_slice() != surfaces {
            self.0.store(Arc::new(surfaces.to_vec()));
        }
    }

    /// The source draws into no surface: it wakes nothing until a frame
    /// draws it again.
    pub fn clear(&self) {
        self.set(&[]);
    }
}
