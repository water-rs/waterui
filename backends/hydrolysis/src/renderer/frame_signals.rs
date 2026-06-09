//! Frame trigger signals shared between the renderer and reactive closures.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::rc::Rc;

use crate::time::Instant;

/// The renderer's frame triggers, shared as one cloneable handle.
///
/// Reactive closures — signal watchers, `Dynamic` content callbacks, animation
/// watchers, navigation controllers, GPU-surface invalidators — hold clones of
/// this handle and request work for an upcoming frame; the frame pump consumes
/// the requests. All state is main-thread-local by design.
///
/// Request kinds, from cheapest to most expensive:
/// - *redraw*: re-render the existing scene (animation tick, caret blink).
/// - *patch*: re-dispatch only the dirty `Dynamic` nodes and re-composite the
///   retained window frame (fine-grained reactive update).
/// - *rebuild*: structural rebuild — re-dispatch the whole window view tree.
#[derive(Clone)]
pub(crate) struct FrameSignals {
    inner: Rc<FrameSignalsInner>,
}

struct FrameSignalsInner {
    redraw_requested: Cell<bool>,
    rebuild_requested: Cell<bool>,
    next_frame_rebuild_requested: Cell<bool>,
    /// Set when a `Dynamic` node's content changed and can be patched in
    /// isolation rather than forcing a full structural rebuild.
    patch_requested: Cell<bool>,
    /// Identities of `Dynamic` nodes whose content changed since the last
    /// frame and must be re-dispatched in isolation on the next patch frame.
    dirty_dynamic_nodes: RefCell<BTreeSet<usize>>,
    /// Monotonic counter of structural rebuilds, used to decide whether a
    /// `Dynamic` content update raced with the rebuild that produced it.
    rebuild_generation: Cell<u64>,
    rebuild_in_progress: Cell<bool>,
    /// The current frame instant, readable from watcher closures that need a
    /// consistent animation clock without capturing the renderer.
    frame_clock: Cell<Instant>,
}

impl FrameSignals {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            inner: Rc::new(FrameSignalsInner {
                redraw_requested: Cell::new(false),
                rebuild_requested: Cell::new(false),
                next_frame_rebuild_requested: Cell::new(false),
                patch_requested: Cell::new(false),
                dirty_dynamic_nodes: RefCell::new(BTreeSet::new()),
                rebuild_generation: Cell::new(0),
                rebuild_in_progress: Cell::new(false),
                frame_clock: Cell::new(now),
            }),
        }
    }

    pub(crate) fn request_redraw(&self) {
        self.inner.redraw_requested.set(true);
    }

    pub(crate) fn take_redraw_request(&self) -> bool {
        self.inner.redraw_requested.replace(false)
    }

    pub(crate) fn request_rebuild(&self) {
        self.inner.rebuild_requested.set(true);
    }

    pub(crate) fn has_rebuild_request(&self) -> bool {
        self.inner.rebuild_requested.get()
    }

    pub(crate) fn take_rebuild_request(&self) -> bool {
        self.inner.rebuild_requested.replace(false)
    }

    /// Request a structural rebuild that must not collapse into the frame
    /// currently being built (used when content discovers mid-dispatch that
    /// the *next* frame needs different structure, e.g. async resource load).
    pub(crate) fn request_next_frame_rebuild(&self) {
        self.inner.next_frame_rebuild_requested.set(true);
        self.inner.redraw_requested.set(true);
    }

    pub(crate) fn take_next_frame_rebuild_request(&self) -> bool {
        self.inner.next_frame_rebuild_requested.replace(false)
    }

    pub(crate) fn has_patch_request(&self) -> bool {
        self.inner.patch_requested.get()
    }

    pub(crate) fn take_patch_request(&self) -> bool {
        self.inner.patch_requested.replace(false)
    }

    /// Take the set of `Dynamic` nodes awaiting an isolated re-dispatch.
    pub(crate) fn take_dirty_dynamic_nodes(&self) -> BTreeSet<usize> {
        core::mem::take(&mut *self.inner.dirty_dynamic_nodes.borrow_mut())
    }

    /// Whether an initial-content update for a `Dynamic` node dispatched at
    /// `render_generation` is already reflected by the rebuild in progress and
    /// must be ignored (the node was just dispatched with exactly this content).
    pub(crate) fn initial_dynamic_content_already_rendered(&self, render_generation: u64) -> bool {
        self.inner.rebuild_in_progress.get()
            && render_generation == self.inner.rebuild_generation.get()
    }

    /// Record a real content change for a `Dynamic` node dispatched at
    /// `render_generation` as a fine-grained reactive patch, unless a rebuild
    /// of a newer generation is in progress that will pick the change up
    /// itself.
    pub(crate) fn mark_dynamic_dirty(&self, identity: usize, render_generation: u64) {
        if self.inner.rebuild_in_progress.get()
            && render_generation != self.inner.rebuild_generation.get()
        {
            return;
        }
        self.inner.dirty_dynamic_nodes.borrow_mut().insert(identity);
        self.inner.patch_requested.set(true);
    }

    /// Enter a structural rebuild: any pending isolated patch is subsumed by
    /// the rebuild, and the rebuild generation advances.
    pub(crate) fn begin_rebuild(&self) {
        self.inner.rebuild_in_progress.set(true);
        self.inner.patch_requested.set(false);
        self.inner.dirty_dynamic_nodes.borrow_mut().clear();
        self.inner.rebuild_generation.set(
            self.inner
                .rebuild_generation
                .get()
                .checked_add(1)
                .expect("hydrolysis renderer rebuild generation overflow"),
        );
    }

    pub(crate) fn finish_rebuild(&self) {
        self.inner.rebuild_in_progress.set(false);
    }

    pub(crate) fn rebuild_generation(&self) -> u64 {
        self.inner.rebuild_generation.get()
    }

    pub(crate) fn set_frame_clock(&self, at: Instant) {
        self.inner.frame_clock.set(at);
    }

    pub(crate) fn frame_clock(&self) -> Instant {
        self.inner.frame_clock.get()
    }
}

#[cfg(test)]
mod tests {
    use super::FrameSignals;
    use crate::time::Instant;

    fn signals() -> FrameSignals {
        FrameSignals::new(Instant::now())
    }

    #[test]
    fn requests_are_consumed_once() {
        let signals = signals();
        signals.request_redraw();
        signals.request_rebuild();
        assert!(signals.has_rebuild_request());
        assert!(signals.take_redraw_request());
        assert!(signals.take_rebuild_request());
        assert!(!signals.take_redraw_request());
        assert!(!signals.take_rebuild_request());
        assert!(!signals.has_rebuild_request());
    }

    #[test]
    fn next_frame_rebuild_also_requests_redraw() {
        let signals = signals();
        signals.request_next_frame_rebuild();
        assert!(signals.take_next_frame_rebuild_request());
        assert!(signals.take_redraw_request());
        assert!(!signals.take_next_frame_rebuild_request());
    }

    #[test]
    fn dynamic_dirty_marking_requests_patch() {
        let signals = signals();
        signals.mark_dynamic_dirty(7, signals.rebuild_generation());
        assert!(signals.has_patch_request());
        assert_eq!(
            signals
                .take_dirty_dynamic_nodes()
                .into_iter()
                .collect::<Vec<_>>(),
            vec![7]
        );
        assert!(signals.take_patch_request());
        assert!(signals.take_dirty_dynamic_nodes().is_empty());
    }

    #[test]
    fn dirty_marking_from_stale_generation_is_ignored_during_rebuild() {
        let signals = signals();
        signals.begin_rebuild();
        let stale_generation = signals.rebuild_generation() - 1;
        signals.mark_dynamic_dirty(1, stale_generation);
        assert!(!signals.has_patch_request());
        assert!(signals.take_dirty_dynamic_nodes().is_empty());

        // A node dispatched by the current rebuild may still mark itself dirty.
        signals.mark_dynamic_dirty(2, signals.rebuild_generation());
        assert!(signals.has_patch_request());

        // Outside a rebuild, generation no longer gates patching.
        signals.finish_rebuild();
        signals.mark_dynamic_dirty(3, stale_generation);
        assert_eq!(signals.take_dirty_dynamic_nodes().len(), 2);
    }

    #[test]
    fn initial_dynamic_content_gating_tracks_rebuild_lifetime() {
        let signals = signals();
        signals.begin_rebuild();
        let generation = signals.rebuild_generation();
        assert!(signals.initial_dynamic_content_already_rendered(generation));
        assert!(!signals.initial_dynamic_content_already_rendered(generation - 1));
        signals.finish_rebuild();
        assert!(!signals.initial_dynamic_content_already_rendered(generation));
    }

    #[test]
    fn begin_rebuild_subsumes_pending_patch() {
        let signals = signals();
        signals.mark_dynamic_dirty(9, signals.rebuild_generation());
        signals.begin_rebuild();
        assert!(!signals.has_patch_request());
        assert!(signals.take_dirty_dynamic_nodes().is_empty());
    }
}
