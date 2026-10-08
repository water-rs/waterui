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
/// - *redraw*: re-present the existing scene (window expose, re-present glue).
/// - *patch/refresh*: re-dispatch any dirty `Dynamic` nodes, then run the
///   always-on full pass — every awake frame re-reads signals, runs layout,
///   and re-encodes the retained tree, so any content change (a reactive
///   value, a scroll offset, a scrollbar drag) takes this path.
///
/// The handle also carries the window's *host wake*, installed once at mount
/// through [`install_host_wake`](Self::install_host_wake): the first request
/// recorded on an idle handle fires it, which is what schedules a frame for a
/// state change made outside one. The pump's `take_*` calls drain the pending
/// state and so re-arm the wake — a burst of requests between frames costs
/// one wake, and an idle handle fires none: no polling, no timers.
#[derive(Clone, Debug)]
pub struct FrameSignals {
    inner: Rc<FrameSignalsInner>,
}

/// The wake a window's host installs on the handle: fired on the edge from
/// no pending request to some pending request, it schedules the frame a
/// request raised outside one needs. Single-threaded like the handle itself —
/// it posts to the host's own scheduler and touches no signal state.
#[derive(Clone)]
struct HostWake(Rc<dyn Fn()>);

impl HostWake {
    fn fire(&self) {
        (self.0)();
    }
}

impl std::fmt::Debug for HostWake {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostWake(..)")
    }
}

#[derive(Debug)]
struct FrameSignalsInner {
    redraw_requested: Cell<bool>,
    /// Set when a `Dynamic` node's content changed and can be patched in
    /// isolation rather than forcing a full structural rebuild.
    patch_requested: Cell<bool>,
    /// The wake the window's host installed via
    /// [`FrameSignals::install_host_wake`]; `None` on a handle no host
    /// drives — the explicitly pumped test runners' windows answer a wake
    /// that does nothing instead.
    host_wake: RefCell<Option<HostWake>>,
    /// Identities of `Dynamic` nodes whose content changed since the last
    /// frame and must be re-dispatched in isolation on the next patch frame.
    dirty_dynamic_nodes: RefCell<BTreeSet<usize>>,
    /// Cache keys of reactive collections whose membership changed since the
    /// last frame and must be reconciled (new items dispatched, removed items
    /// evicted) in isolation on the next patch frame.
    dirty_collections: RefCell<BTreeSet<usize>>,
    /// Monotonic counter of structural rebuilds, used to decide whether a
    /// `Dynamic` content update raced with the rebuild that produced it.
    rebuild_generation: Cell<u64>,
    rebuild_in_progress: Cell<bool>,
    /// The current frame instant, readable from watcher closures that need a
    /// consistent animation clock without capturing the renderer.
    frame_clock: Cell<Instant>,
}

impl FrameSignals {
    /// Creates a fresh handle with no pending requests and `now` as the
    /// initial frame clock.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            inner: Rc::new(FrameSignalsInner {
                redraw_requested: Cell::new(false),
                patch_requested: Cell::new(false),
                host_wake: RefCell::new(None),
                dirty_dynamic_nodes: RefCell::new(BTreeSet::new()),
                dirty_collections: RefCell::new(BTreeSet::new()),
                rebuild_generation: Cell::new(0),
                rebuild_in_progress: Cell::new(false),
                frame_clock: Cell::new(now),
            }),
        }
    }

    /// Installs the host wake the window's runner supplies — called once
    /// per window at mount, on the construction point every runner shares.
    ///
    /// The wake runs on the edge from *no pending request* to *some
    /// pending request*: the first `request_*`/`mark_*` call to land on an
    /// idle handle fires it, and the pump draining the state back to empty
    /// re-arms it. A request already pending when the wake is installed
    /// fires it now — the edge it never saw still owes the host a frame.
    pub fn install_host_wake(&self, wake: Rc<dyn Fn()>) {
        *self.inner.host_wake.borrow_mut() = Some(HostWake(wake));
        if self.has_pending_request() {
            self.fire_host_wake();
        }
    }

    /// Whether any frame request is pending — the *some pending request*
    /// side of the edge the host wake fires on. A host that suppresses its
    /// wake inside its own frame transaction counts this into the
    /// transaction's continuation instead, so a request raised
    /// mid-transaction is never lost.
    #[must_use]
    pub fn has_pending_request(&self) -> bool {
        self.inner.redraw_requested.get() || self.inner.patch_requested.get()
    }

    /// Records a request and fires the host wake when it moved the handle
    /// off idle. Every `request_*`/`mark_*` method routes through here, so
    /// the edge is observed exactly once per burst — a request landing on
    /// a handle that already has one pending joins it without re-firing.
    fn record_request(&self, record: impl FnOnce(&FrameSignalsInner)) {
        let was_pending = self.has_pending_request();
        record(&self.inner);
        if !was_pending {
            self.fire_host_wake();
        }
    }

    /// Runs the installed wake — cloned out of the `RefCell` first, so a
    /// wake that records its own request inside its post never fires under
    /// a held borrow.
    fn fire_host_wake(&self) {
        let wake = self.inner.host_wake.borrow().clone();
        if let Some(wake) = wake {
            wake.fire();
        }
    }

    /// Requests a re-render of the existing scene on the next frame (the
    /// cheapest request kind: animation tick, caret blink).
    pub fn request_redraw(&self) {
        self.record_request(|inner| inner.redraw_requested.set(true));
    }

    /// Consumes the pending redraw request, returning whether one was set.
    ///
    /// The flag resets to `false`, so each request is observed exactly once
    /// by the frame pump.
    #[must_use]
    pub fn take_redraw_request(&self) -> bool {
        self.inner.redraw_requested.replace(false)
    }

    /// Requests a re-flush of the retained render tree on the next frame: a
    /// reactive *value* changed (text content, color, a leaf's intrinsic size), so
    /// the tree structure is unchanged but it must re-read its signals, re-run
    /// layout, and re-encode.
    ///
    /// Routed through the same window-refresh path as a `Dynamic` patch
    /// ([`take_patch_request`](Self::take_patch_request) observes it) but with an
    /// empty dirty-node set, so the patch step is a no-op and the change is reflected
    /// purely by the always-on relayout + reflush. This is the common reactive
    /// update — far cheaper than a structural rebuild, which re-runs the whole view
    /// `body()`.
    pub fn request_refresh(&self) {
        self.record_request(|inner| inner.patch_requested.set(true));
    }

    /// Returns whether at least one dirty `Dynamic` node awaits an isolated
    /// patch, without consuming the request.
    #[must_use]
    pub fn has_patch_request(&self) -> bool {
        self.inner.patch_requested.get()
    }

    /// Consumes the pending patch request, returning whether one was set.
    ///
    /// Only the flag is consumed; the dirty-node set is taken separately via
    /// [`take_dirty_dynamic_nodes`](Self::take_dirty_dynamic_nodes).
    #[must_use]
    pub fn take_patch_request(&self) -> bool {
        self.inner.patch_requested.replace(false)
    }

    /// Take the set of `Dynamic` nodes awaiting an isolated re-dispatch.
    #[must_use]
    pub fn take_dirty_dynamic_nodes(&self) -> BTreeSet<usize> {
        core::mem::take(&mut *self.inner.dirty_dynamic_nodes.borrow_mut())
    }

    /// Take the set of reactive collections awaiting an isolated reconcile.
    #[must_use]
    pub fn take_dirty_collections(&self) -> BTreeSet<usize> {
        core::mem::take(&mut *self.inner.dirty_collections.borrow_mut())
    }

    /// Whether an initial-content update for a `Dynamic` node dispatched at
    /// `render_generation` is already reflected by the rebuild in progress and
    /// must be ignored (the node was just dispatched with exactly this content).
    #[must_use]
    pub fn initial_dynamic_content_already_rendered(&self, render_generation: u64) -> bool {
        self.inner.rebuild_in_progress.get()
            && render_generation == self.inner.rebuild_generation.get()
    }

    /// Record a real content change for a `Dynamic` node dispatched at
    /// `render_generation` as a fine-grained reactive patch, unless a rebuild
    /// of a newer generation is in progress that will pick the change up
    /// itself.
    pub fn mark_dynamic_dirty(&self, identity: usize, render_generation: u64) {
        if self.inner.rebuild_in_progress.get()
            && render_generation != self.inner.rebuild_generation.get()
        {
            return;
        }
        self.record_request(|inner| {
            inner.dirty_dynamic_nodes.borrow_mut().insert(identity);
            inner.patch_requested.set(true);
        });
    }

    /// Record a membership change for a reactive collection captured at
    /// `render_generation` as a fine-grained reactive patch, unless a rebuild
    /// of a newer generation is in progress that will recapture it itself.
    ///
    /// Mirrors [`mark_dynamic_dirty`](Self::mark_dynamic_dirty): a collection's
    /// `Views::watch` fires once immediately on registration (the initial
    /// snapshot during the capturing rebuild); that fire is ignored by the
    /// caller, and only later real changes reach this method.
    pub fn mark_collection_dirty(&self, cache_key: usize, render_generation: u64) {
        if self.inner.rebuild_in_progress.get()
            && render_generation != self.inner.rebuild_generation.get()
        {
            return;
        }
        self.record_request(|inner| {
            inner.dirty_collections.borrow_mut().insert(cache_key);
            inner.patch_requested.set(true);
        });
    }

    /// Enter a structural rebuild: any pending isolated patch is subsumed by
    /// the rebuild, and the rebuild generation advances.
    ///
    /// # Panics
    ///
    /// Panics if the rebuild generation counter overflows.
    pub fn begin_rebuild(&self) {
        self.inner.rebuild_in_progress.set(true);
        self.inner.patch_requested.set(false);
        self.inner.dirty_dynamic_nodes.borrow_mut().clear();
        self.inner.dirty_collections.borrow_mut().clear();
        self.inner.rebuild_generation.set(
            self.inner
                .rebuild_generation
                .get()
                .checked_add(1)
                .expect("hydrolysis renderer rebuild generation overflow"),
        );
    }

    /// Leaves the structural rebuild entered by
    /// [`begin_rebuild`](Self::begin_rebuild); generation gating of
    /// [`mark_dynamic_dirty`](Self::mark_dynamic_dirty) stops applying.
    pub fn finish_rebuild(&self) {
        self.inner.rebuild_in_progress.set(false);
    }

    /// Returns the current structural rebuild generation.
    ///
    /// `Dynamic` dispatch captures this value so later content updates can be
    /// attributed to the rebuild that produced the node.
    #[must_use]
    pub fn rebuild_generation(&self) -> u64 {
        self.inner.rebuild_generation.get()
    }

    /// Sets the frame clock; the frame pump calls this once at the start of
    /// each frame so every closure in that frame observes the same instant.
    pub fn set_frame_clock(&self, at: Instant) {
        self.inner.frame_clock.set(at);
    }

    /// Returns the instant of the frame currently being produced, for watcher
    /// closures that need a consistent animation clock.
    #[must_use]
    pub fn frame_clock(&self) -> Instant {
        self.inner.frame_clock.get()
    }
}

#[cfg(test)]
mod tests {
    use super::{BTreeSet, FrameSignals};
    use crate::time::Instant;
    use std::cell::Cell;
    use std::rc::Rc;

    fn signals() -> FrameSignals {
        FrameSignals::new(Instant::now())
    }

    /// A wake closure plus the counter it bumps — the observer the host
    /// wake contract tests assert against.
    fn counting_wake() -> (Rc<dyn Fn()>, Rc<Cell<u32>>) {
        let fires = Rc::new(Cell::new(0u32));
        let wake = {
            let fires = fires.clone();
            move || fires.set(fires.get() + 1)
        };
        (Rc::new(wake), fires)
    }

    #[test]
    fn requests_are_consumed_once() {
        let signals = signals();
        signals.request_redraw();
        signals.request_refresh();
        assert!(signals.has_pending_request());
        assert!(signals.take_redraw_request());
        assert!(signals.take_patch_request());
        assert!(!signals.take_redraw_request());
        assert!(!signals.take_patch_request());
        assert!(!signals.has_pending_request());
    }

    /// The host wake contract the runners rely on (water-rs/waterui#2286):
    /// a burst of requests across every kind between frames fires it once,
    /// a request landing while one is still pending never re-fires, and
    /// the pump's `take_*` calls re-arm the edge by draining the state.
    #[test]
    fn host_wake_fires_once_per_pending_edge() {
        let signals = signals();
        let (wake, fires) = counting_wake();
        signals.install_host_wake(wake);
        assert_eq!(fires.get(), 0, "installing on an idle handle fires nothing");

        signals.request_redraw();
        signals.request_refresh();
        signals.mark_dynamic_dirty(1, signals.rebuild_generation());
        signals.mark_collection_dirty(2, signals.rebuild_generation());
        assert_eq!(fires.get(), 1, "the burst's first request fires the wake");

        // Redraw drained but the patch request is still pending: no edge.
        assert!(signals.take_redraw_request());
        signals.request_refresh();
        assert_eq!(fires.get(), 1);

        // Fully drained, the next request is a new edge.
        assert!(signals.take_patch_request());
        signals.request_refresh();
        assert_eq!(fires.get(), 2, "drained state re-arms the wake");
    }

    /// A `mark_*` generation-gated out of recording — and non-request
    /// state changes like the frame clock or a rebuild's begin/finish —
    /// never fire the wake.
    #[test]
    fn host_wake_never_fires_on_a_no_op() {
        let signals = signals();
        let (wake, fires) = counting_wake();
        signals.install_host_wake(wake);

        signals.set_frame_clock(Instant::now());
        signals.begin_rebuild();
        let stale_generation = signals.rebuild_generation() - 1;
        signals.mark_dynamic_dirty(1, stale_generation);
        signals.mark_collection_dirty(1, stale_generation);
        signals.finish_rebuild();
        assert_eq!(fires.get(), 0);
    }

    /// A request already pending when the host installs its wake still
    /// fires it — the edge the install never saw owes the host a frame.
    #[test]
    fn host_wake_fires_on_install_over_a_pending_request() {
        let signals = signals();
        signals.request_refresh();
        let (wake, fires) = counting_wake();
        signals.install_host_wake(wake);
        assert_eq!(fires.get(), 1);
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
        assert_eq!(signals.take_dirty_dynamic_nodes(), BTreeSet::new());
    }

    #[test]
    fn collection_dirty_marking_requests_patch() {
        let signals = signals();
        signals.mark_collection_dirty(42, signals.rebuild_generation());
        assert!(signals.has_patch_request());
        assert_eq!(
            signals
                .take_dirty_collections()
                .into_iter()
                .collect::<Vec<_>>(),
            vec![42]
        );
        assert!(signals.take_patch_request());
        assert_eq!(signals.take_dirty_collections(), BTreeSet::new());
    }

    #[test]
    fn begin_rebuild_subsumes_pending_collection_patch() {
        let signals = signals();
        signals.mark_collection_dirty(7, signals.rebuild_generation());
        signals.begin_rebuild();
        assert!(!signals.has_patch_request());
        assert_eq!(signals.take_dirty_collections(), BTreeSet::new());
    }

    #[test]
    fn dirty_marking_from_stale_generation_is_ignored_during_rebuild() {
        let signals = signals();
        signals.begin_rebuild();
        let stale_generation = signals.rebuild_generation() - 1;
        signals.mark_dynamic_dirty(1, stale_generation);
        assert!(!signals.has_patch_request());
        assert_eq!(signals.take_dirty_dynamic_nodes(), BTreeSet::new());

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
        assert_eq!(signals.take_dirty_dynamic_nodes(), BTreeSet::new());
    }
}
