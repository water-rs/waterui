//! A layer's layout size: the size its host lays it out at, exposed to the
//! layer's recordings as a nami signal.

use std::cell::Cell;
use std::rc::Rc;

use kurbo::Size;
use nami_core::watcher::{Context, Metadata, WatcherManager, WatcherManagerGuard};
use nami_core::{Signal, SignalIdentity};

use crate::animation::Animation;

/// The size a layer is laid out at, in the layer's content coordinates.
///
/// Every layer has one; the host drives it from layout through the
/// target's layer edit, and a recording made for the layer reads it from
/// [`Recorder::layout_size`](crate::Recorder::layout_size). It is a signal,
/// so size-dependent geometry binds to it like any other value: a resize
/// updates only the commands that reference it, without re-recording. A
/// size the host sets under an [`Animation`] animates those operands.
///
/// It reads [`Size::ZERO`] until the host sets it.
#[derive(Clone)]
pub struct LayoutSize {
    inner: Rc<Inner>,
}

struct Inner {
    value: Cell<Size>,
    watchers: WatcherManager<Size>,
}

impl std::fmt::Debug for LayoutSize {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LayoutSize")
            .field(&self.inner.value.get())
            .finish()
    }
}

impl Default for LayoutSize {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutSize {
    /// A layer's size before its host sets one — a target owns exactly
    /// one per layer and hands it to every recording for that layer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Rc::new(Inner {
                value: Cell::new(Size::ZERO),
                watchers: WatcherManager::new(),
            }),
        }
    }

    /// Sets the size, notifying bound recordings when it changed. The
    /// context's metadata travels with the change, so an [`Animation`] in
    /// it animates every operand bound to the size. Built by
    /// [`change`](Self::change), or by a nami context the host's own
    /// layout signal carries.
    pub fn set(&self, change: &Context<Size>) {
        if *change.value() == self.inner.value.get() {
            return;
        }
        self.inner.value.set(*change.value());
        self.inner.watchers.notify(change);
    }

    /// The change a transaction makes: `size` under `animation`, if any —
    /// what [`set`](Self::set) consumes, so the host can put a resize
    /// under an animation the same way a bound signal's change carries
    /// one.
    #[must_use]
    pub fn change(size: Size, animation: Option<Animation>) -> Context<Size> {
        let context = Context::new(size, Metadata::new());
        match animation {
            Some(animation) => context.with(animation),
            None => context,
        }
    }
}

impl Signal for LayoutSize {
    type Output = Size;
    type Guard = WatcherManagerGuard<Size>;

    fn snapshot(&self) -> Size {
        self.inner.value.get()
    }

    fn identity(&self) -> Option<SignalIdentity> {
        Some(SignalIdentity::from_rc(&self.inner))
    }

    fn watch(&self, watcher: impl Fn(Context<Size>) + 'static) -> Self::Guard {
        self.inner.watchers.register_as_guard(watcher)
    }
}
