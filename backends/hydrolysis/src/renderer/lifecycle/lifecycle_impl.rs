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
/// A root signal's `SignalIdentity` derives from its shared allocation address,
/// so the entry holds a clone of the signal itself: that pins the allocation,
/// making address-reuse ABA (a pruned signal's address resurfacing as a
/// different live signal under the same key) impossible while the entry exists.
/// Derived signals (`map`/`zip`/`WithMetadata`) mix a call-site discriminator
/// into the address, so their keys are hashes rather than pinned addresses; a
/// cross-type collision would silently drop a subscription, so `mark_seen`
/// fast-fails when the key's recorded signal type does not match.
#[derive(Default)]
pub(crate) struct SignalWatchRegistry {
    entries: FxHashMap<usize, SignalWatchEntry>,
    generation: u64,
}

struct SignalWatchEntry {
    /// Clone of the watched signal; pins the identity allocation (see type docs).
    _signal: Box<dyn Any>,
    /// Concrete type of the subscribed signal, used to detect identity-key
    /// collisions between different signal types.
    signal_type: core::any::TypeId,
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
    ///
    /// # Panics
    ///
    /// Panics when the identity key is already held by a *different* signal
    /// type — a derived-identity hash collision that would otherwise silently
    /// swallow the second signal's updates.
    pub(crate) fn mark_seen(&mut self, identity: usize, signal_type: core::any::TypeId) -> bool {
        self.entries.get_mut(&identity).is_some_and(|entry| {
            assert!(
                entry.signal_type == signal_type,
                "hydrolysis renderer: signal identity {identity:#x} is shared by two different signal types (derived-identity hash collision)"
            );
            entry.last_seen = self.generation;
            true
        })
    }

    /// Records a fresh subscription for `identity`, alive until the signal goes
    /// a whole frame without being read.
    pub(crate) fn insert(
        &mut self,
        identity: usize,
        signal_type: core::any::TypeId,
        signal: Box<dyn Any>,
        guard: Retain,
    ) {
        self.entries.insert(
            identity,
            SignalWatchEntry {
                _signal: signal,
                signal_type,
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
