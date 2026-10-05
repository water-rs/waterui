//! The engine's one wake mechanism: each surface's host wake-up, and the
//! fan-out of an event a backend source causes to the surfaces it draws
//! into.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use arc_swap::ArcSwap;

use crate::backend::Visibility;

/// A host's wake-up callback. It may run on any thread a wake starts on —
/// the UI thread, the render thread, a backend completion, a producer's or
/// a filter parameter's thread — so it is `Send` to cross threads and
/// `Sync` to be shared across them, on every target.
type Wake = dyn Fn() + Send + Sync;

/// Where a surface's wake stands between its renders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum WakeState {
    /// The next wake reaches the host.
    Armed,
    /// The host was woken and the render that answers it has not drained
    /// the surface yet: every wake until then is part of that frame.
    Disarmed,
    /// A [`FrameScope`] is open and nothing has asked for a frame since
    /// it opened.
    InFrame,
    /// A [`FrameScope`] is open and something asked for a frame since it
    /// opened: the scope's render answers it, or the scope's end wakes
    /// the host for it.
    InFrameOwed,
}

impl WakeState {
    const fn from_bits(bits: u8) -> Self {
        match bits {
            0 => Self::Armed,
            1 => Self::Disarmed,
            2 => Self::InFrame,
            3 => Self::InFrameOwed,
            _ => panic!("a surface wake state holds only the four WakeState values"),
        }
    }

    /// A wake: it reaches the host only from `Armed`; inside a frame scope
    /// it becomes the scope's debt.
    const fn woken(self) -> Self {
        match self {
            Self::Armed | Self::Disarmed => Self::Disarmed,
            Self::InFrame | Self::InFrameOwed => Self::InFrameOwed,
        }
    }

    /// The host opens a frame: it answers every wake delivered before it.
    fn opened(self) -> Self {
        match self {
            Self::Armed | Self::Disarmed => Self::InFrame,
            Self::InFrame | Self::InFrameOwed => {
                panic!("a surface has at most one open FrameScope")
            }
        }
    }

    /// The frame scope ends. A render inside it already left it; without
    /// one, a debt becomes a delivered wake and no debt re-arms.
    const fn closed(self) -> Self {
        match self {
            Self::Armed | Self::InFrame => Self::Armed,
            Self::Disarmed | Self::InFrameOwed => Self::Disarmed,
        }
    }
}

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
    /// The [`WakeState`]. Every transition is a read-modify-write with
    /// `AcqRel`, a wake that leaves the state unchanged included: a source
    /// that writes its `dirty` flag and then finds the wake already taken
    /// releases that write into this atomic, and the render's re-arm
    /// acquires it, so the render that answers the wake sees the flag on
    /// any thread and any memory model.
    state: AtomicU8,
    /// Whether the host last announced the surface visible.
    visible: AtomicBool,
}

impl std::fmt::Debug for SurfaceWaker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SurfaceWaker")
            .field(
                "state",
                &WakeState::from_bits(self.state.load(Ordering::Acquire)),
            )
            .field("visibility", &self.visibility())
            .finish_non_exhaustive()
    }
}

impl SurfaceWaker {
    /// A visible surface's wake-up through the host's `wake`.
    pub(crate) fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self {
            wake: Box::new(wake),
            state: AtomicU8::new(WakeState::Armed as u8),
            visible: AtomicBool::new(true),
        }
    }

    /// Applies `transition` to the state and returns the state it left.
    fn transition(&self, transition: impl Fn(WakeState) -> WakeState) -> WakeState {
        let mut current = WakeState::Armed;
        loop {
            match self.state.compare_exchange_weak(
                current as u8,
                transition(current) as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return current,
                Err(actual) => current = WakeState::from_bits(actual),
            }
        }
    }

    /// Wakes the host once if armed, then disarms until the surface next
    /// renders — unless the surface is hidden. Inside a frame scope the
    /// wake is owed to the scope instead.
    pub(crate) fn wake(&self) {
        if self.visible.load(Ordering::Acquire)
            && self.transition(WakeState::woken) == WakeState::Armed
        {
            (self.wake)();
        }
    }

    /// Re-arms the wake after the surface participated in a render, which
    /// answers whatever was owed.
    pub(crate) fn arm(&self) {
        self.state.swap(WakeState::Armed as u8, Ordering::AcqRel);
    }

    /// Opens the frame scope.
    fn open_frame(&self) {
        self.transition(WakeState::opened);
    }

    /// Ends the frame scope, waking the host for what it owes.
    fn close_frame(&self) {
        if self.transition(WakeState::closed) == WakeState::InFrameOwed
            && self.visible.load(Ordering::Acquire)
        {
            (self.wake)();
        }
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
    /// draws was hidden. Inside a frame scope the request is owed to the
    /// scope like any other wake.
    pub(crate) fn show(&self) {
        self.visible.store(true, Ordering::Release);
        if !matches!(
            self.transition(WakeState::woken),
            WakeState::InFrame | WakeState::InFrameOwed
        ) {
            (self.wake)();
        }
    }

    /// The surface is gone: its wakes — a backend completion landing
    /// after drop, a leaked layer's queued op, a producer or filter whose
    /// wakes still list it — stay silent forever.
    pub(crate) fn retire(&self) {
        self.hide();
    }
}

/// A surface's frame, from [`Surface::begin_frame`](crate::Surface::begin_frame)
/// until the scope is dropped: the host is building a frame that ends in
/// [`Engine::render`](crate::Engine::render).
///
/// - Opening the scope takes over every wake the host was already given:
///   the frame it opens answers them.
/// - While the scope is open and before its render, a wake on the
///   surface's behalf — the host's own edits, a bound signal, a producer,
///   filter or backend completion on any thread, the surface becoming
///   visible — does not call the host. It records that a frame is owed.
/// - A render that draws the surface while the scope is open answers
///   everything owed: it clears the debt and re-arms the wake. From then
///   on, though the scope is still open, a wake reaches the host as usual
///   and asks for the next frame; dropping the scope changes nothing.
/// - Dropping the scope without such a render — the host's frame failed
///   or returned early — wakes the host once if a frame is owed and the
///   surface is visible, and re-arms the wake otherwise.
///
/// A wake is therefore deferred, never lost. A surface has at most one
/// open scope: opening a second panics.
#[derive(Debug)]
#[must_use = "dropping the scope ends the frame; keep it until the frame's render"]
pub struct FrameScope(Arc<SurfaceWaker>);

impl FrameScope {
    /// Opens the frame scope on the surface's wake.
    pub(crate) fn open(waker: &Arc<SurfaceWaker>) -> Self {
        waker.open_frame();
        Self(Arc::clone(waker))
    }
}

impl Drop for FrameScope {
    fn drop(&mut self) {
        self.0.close_frame();
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
    ///
    /// A source requesting a redraw marks its `dirty` flag with an `AcqRel`
    /// read-modify-write before it calls this (see [`Self::set`]).
    pub fn wake(&self) {
        for waker in self.0.load_full().iter() {
            waker.wake();
        }
    }

    /// Replaces the surfaces the source draws into, after the frame
    /// consumed the source's `dirty` flag. Setting the surfaces it already
    /// has allocates nothing.
    ///
    /// A surface the list gains is woken when `dirty` is set: the request
    /// that set it after the frame consumed it woke only the surfaces the
    /// list held then. The flag is read with an `AcqRel` read-modify-write
    /// after the list is replaced, so a request that marks it after that
    /// read acquires the new list and wakes the gained surfaces itself.
    pub fn set(&self, surfaces: &[CompletionWaker], dirty: &AtomicBool) {
        if self.0.load().as_slice() == surfaces {
            return;
        }
        let previous = self.0.swap(Arc::new(surfaces.to_vec()));
        if dirty.fetch_or(false, Ordering::AcqRel) {
            for waker in surfaces.iter().filter(|waker| !previous.contains(waker)) {
                waker.wake();
            }
        }
    }

    /// The source draws into no surface: it wakes nothing until a frame
    /// draws it again.
    pub fn clear(&self) {
        if !self.0.load().is_empty() {
            self.0.store(Arc::new(Vec::new()));
        }
    }
}
