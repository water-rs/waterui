use super::*;
use core::any::Any;
use rustc_hash::FxHashMap;

#[derive(Default)]
pub(crate) struct LifecycleState {
    pub(crate) current_frame_retain: Vec<Retain>,
    pub(crate) previous_frame_retain: Vec<Retain>,
    pub(crate) signal_watches: SignalWatchRegistry,
}

/// Cross-frame registry of the per-signal refresh subscriptions created by
/// `HydrolysisRenderer::watch_signal`.
///
/// Every frame re-reads every reactive input of the retained tree. Without this
/// registry each read re-subscribed a fresh watcher (and dropped last frame's),
/// churning one allocation + registration per signal read per frame. Instead, a
/// signal with a stable `SignalIdentity` subscribes once on first sight and the
/// subscription is reused as long as the signal keeps being read; an entry not
/// seen for a whole frame belongs to a subtree that no longer flushes (patched
/// away or scrolled out of a virtualized window) and is pruned.
///
/// `SignalIdentity` derives from the signal's shared allocation address, so the
/// entry holds a clone of the signal itself: that pins the allocation, making
/// address-reuse ABA (a pruned signal's address resurfacing as a different live
/// signal under the same key) impossible while the entry exists.
#[derive(Default)]
pub(crate) struct SignalWatchRegistry {
    entries: FxHashMap<usize, SignalWatchEntry>,
    generation: u64,
}

struct SignalWatchEntry {
    /// Clone of the watched signal; pins the identity allocation (see type docs).
    _signal: Box<dyn Any>,
    /// The watcher subscription, cancelled on drop.
    _guard: Retain,
    last_seen: u64,
}

impl SignalWatchRegistry {
    fn begin_frame(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("hydrolysis renderer: signal watch generation overflow");
    }

    fn finish_frame(&mut self) {
        let generation = self.generation;
        self.entries
            .retain(|_, entry| entry.last_seen == generation);
    }

    /// Marks an existing subscription for `identity` as read this frame.
    /// Returns `false` when no subscription exists yet.
    pub(crate) fn mark_seen(&mut self, identity: usize) -> bool {
        self.entries.get_mut(&identity).is_some_and(|entry| {
            entry.last_seen = self.generation;
            true
        })
    }

    /// Records a fresh subscription for `identity`, alive until the signal goes
    /// a whole frame without being read.
    pub(crate) fn insert(&mut self, identity: usize, signal: Box<dyn Any>, guard: Retain) {
        self.entries.insert(
            identity,
            SignalWatchEntry {
                _signal: signal,
                _guard: guard,
                last_seen: self.generation,
            },
        );
    }
}

pub(crate) struct DeferredLifeCycleHook {
    pub(crate) env: Environment,
    pub(crate) hook: LifeCycleHook,
}

impl LifecycleState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.previous_frame_retain.clear();
        self.current_frame_retain.clear();
        self.signal_watches.begin_frame();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.previous_frame_retain = core::mem::take(&mut self.current_frame_retain);
        self.signal_watches.finish_frame();
    }
}

impl DeferredLifeCycleHook {
    pub(crate) fn new(hook: LifeCycleHook, env: Environment) -> Self {
        Self { env, hook }
    }

    pub(crate) fn call(self) {
        self.hook.handle(&self.env);
    }
}
