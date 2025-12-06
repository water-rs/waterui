//! Reactive helpers for render nodes.

extern crate alloc;

use alloc::sync::Arc;
use core::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
};

use nami::{Computed, Signal, watcher::BoxWatcherGuard};

/// Wraps a `Computed<T>` and tracks whether its value changed since the last refresh.
pub struct NodeSignal<T>
where
    T: Clone + 'static,
{
    computed: Computed<T>,
    current: T,
    dirty: Arc<AtomicBool>,
    _guard: BoxWatcherGuard,
}

impl<T> NodeSignal<T>
where
    T: Clone + 'static,
{
    /// Creates a new node signal from the provided computed value.
    #[must_use]
    pub fn new(computed: Computed<T>) -> Self {
        let current = computed.get();
        let dirty = Arc::new(AtomicBool::new(false));
        let dirty_flag = dirty.clone();
        let guard = computed.watch(move |_| {
            dirty_flag.store(true, Ordering::Relaxed);
        });
        Self {
            computed,
            current,
            dirty,
            _guard: guard,
        }
    }

    /// Returns the cached value.
    #[must_use]
    pub const fn current(&self) -> &T {
        &self.current
    }

    /// Refreshes the cached value if it changed, returning `true` when updated.
    pub fn refresh(&mut self) -> bool {
        if self.dirty.swap(false, Ordering::Relaxed) {
            self.current = self.computed.get();
            true
        } else {
            false
        }
    }
}

impl<T> fmt::Debug for NodeSignal<T>
where
    T: Clone + fmt::Debug + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeSignal")
            .field("current", &self.current)
            .field("dirty", &self.dirty.load(Ordering::Relaxed))
            .finish()
    }
}
