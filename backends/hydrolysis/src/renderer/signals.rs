//! Reactive inputs: signal watching and animated-value sampling that bind
//! `WaterUI` signals to frame triggers and the animation controller.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use rustc_hash::FxHashMap;

type SignalUpdateHandler<T> = Rc<dyn Fn(nami::watcher::Context<T>)>;

/// Subscribes `callback` to `signal`, returning the [`Retain`] that keeps the
/// subscription alive.
///
/// Some signals emit synchronously while `watch` registers — a collection's
/// populated emission reports the current contents as an insertion. That
/// registration-time emission echoes the value a `snapshot()` beside the call
/// reads, so it must not count as an update: the callback arms only after
/// `watch` returns.
fn subscribe_signal<S>(
    signal: &S,
    callback: impl Fn(nami::watcher::Context<S::Output>) + 'static,
) -> Retain
where
    S: Signal + Clone + 'static,
{
    let armed = Rc::new(Cell::new(false));
    let armed_for_watch = Rc::clone(&armed);
    let guard = signal.watch(move |update| {
        if armed_for_watch.get() {
            callback(update);
        }
    });
    armed.set(true);
    Retain::new(guard)
}

/// The dedup-and-sweep signal subscription store a [`LayoutDependencies`]
/// identity set is built on. A signal with a stable `SignalIdentity`
/// subscribes once on first sight and the subscription is reused as long as
/// every pass keeps reading it; an entry a whole sweep window leaves unread
/// is pruned at `finish_frame`.
///
/// A root signal's `SignalIdentity` derives from its shared allocation
/// address, so the entry holds a clone of the signal itself: that pins the
/// allocation, making address-reuse ABA (a pruned signal's address
/// resurfacing as a different live signal under the same key) impossible
/// while the entry exists. Derived signals (`map`/`zip`/`WithMetadata`) mix
/// a call-site discriminator into the address, so their keys are hashes
/// rather than pinned addresses; a cross-type collision would silently drop
/// a subscription, so `mark_seen` fast-fails when the key's recorded signal
/// type does not match.
#[derive(Default)]
pub(super) struct SignalWatchRegistry {
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
    /// Opens a sweep window: entries marked seen in it survive the matching
    /// [`Self::finish_frame`].
    const fn begin_frame(&mut self) {
        self.generation = self
            .generation
            .checked_add(1)
            .expect("hydrolysis renderer: signal watch generation overflow");
    }

    /// Closes the sweep window, pruning every entry no `mark_seen` reached
    /// since [`Self::begin_frame`].
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
    fn mark_seen(&mut self, identity: usize, signal_type: core::any::TypeId) -> bool {
        self.entries.get_mut(&identity).is_some_and(|entry| {
            assert!(
                entry.signal_type == signal_type,
                "hydrolysis renderer: signal identity {identity:#x} is shared by two different signal types (derived-identity hash collision)"
            );
            entry.last_seen = self.generation;
            true
        })
    }

    /// Records a fresh subscription for `identity`, alive until it goes a
    /// whole sweep window — a `BuiltSubview`'s layout pass — without being
    /// read.
    fn insert(
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

/// The signal dependencies of one `BuiltSubview`'s cached layout.
///
/// `BuiltSubview::layout_if_needed` runs `RenderNode::layout` only when the
/// inputs it caches against changed; once a pass is skipped, the frame watch
/// registry drops the pass's subscriptions after one unread frame, so a write
/// to a signal only that pass read never reaches the sub-view again — the
/// cached layout stays stale. The dependency set lives beside the layout it
/// describes: while a pass runs it is installed in
/// [`HydroState::layout_dependencies`], where [`SemanticCore::watch_signal`]
/// and [`HydroState::measure_signal`] record every read, and it keeps each
/// signal subscribed for as long as the cached layout lives. An update marks
/// `dirty` and requests one refresh, which the owning `BuiltSubview` folds
/// into `needs_layout` — the next flush re-runs exactly that sub-view's
/// layout.
///
/// `identities` is the same [`SignalWatchRegistry`] the frame pump uses: a
/// signal with a stable identity subscribes once and the subscription is
/// reused while every pass keeps reading it, so a steady-state pass allocates
/// and subscribes nothing. Identity-less signals have no key to reuse under —
/// each pass drops their previous subscriptions and re-subscribes.
pub(super) struct LayoutDependencies {
    /// Identity-stable dependencies: subscribed on first read and kept —
    /// pruned back to what the latest pass read by `finish_pass`.
    identities: SignalWatchRegistry,
    /// The identity-less dependencies of the latest pass: subscribed fresh
    /// every pass, so subscriptions are replaced wholesale rather than
    /// reused.
    anonymous: Vec<Retain>,
    /// Set by a dependency's subscription when the signal updates. The owning
    /// `BuiltSubview` reads it once per flush: `true` forces its layout pass
    /// to re-run, so the next flush's `begin_pass` clears it.
    dirty: Rc<Cell<bool>>,
    /// The sub-view's own cell — the node a dependency's update marks for
    /// re-layout (the `Layout::watch_invalidation` row of §B.1).
    cell: Weak<NodeCell>,
}

impl LayoutDependencies {
    /// An empty set for a sub-view whose layout has not run yet.
    pub(super) fn new(cell: Weak<NodeCell>) -> Self {
        Self {
            identities: SignalWatchRegistry::default(),
            anonymous: Vec::new(),
            dirty: Rc::new(Cell::new(false)),
            cell,
        }
    }

    /// Whether a dependency delivered an update since the last pass — the
    /// invalidation mark `layout_if_needed` folds into `needs_layout`.
    pub(super) fn is_dirty(&self) -> bool {
        self.dirty.get()
    }

    /// Opens a collection pass: a clean dirty flag, a fresh sweep window for
    /// the identity set, and the previous pass's identity-less subscriptions
    /// dropped (the pass re-subscribes each one it still reads).
    pub(super) fn begin_pass(&mut self) {
        self.dirty.set(false);
        self.identities.begin_frame();
        self.anonymous.clear();
    }

    /// Closes the pass: identity subscriptions this pass did not read are
    /// swept — the layout they fed is gone.
    pub(super) fn finish_pass(&mut self) {
        self.identities.finish_frame();
    }

    /// Records one signal read as a dependency of the cached layout this set
    /// describes.
    fn watch<S>(&mut self, signal: &S)
    where
        S: Signal + Clone + 'static,
    {
        if let Some(identity) = signal.identity() {
            let key = identity.raw();
            let signal_type = core::any::TypeId::of::<S>();
            if self.identities.mark_seen(key, signal_type) {
                return;
            }
            let guard = self.subscribe(signal);
            self.identities
                .insert(key, signal_type, Box::new(signal.clone()), guard);
        } else {
            self.anonymous.push(self.subscribe(signal));
        }
    }

    /// One subscription marking the set dirty on an update. The owning cell
    /// is marked for re-layout only on the dirty edge — the owning
    /// `BuiltSubview` folds the flag into `needs_layout` on that frame, so a
    /// busy signal read by a sub-view whose layout stays cached (hidden
    /// off-screen, or yet to settle) requests one frame, not one per write.
    fn subscribe<S>(&self, signal: &S) -> Retain
    where
        S: Signal + Clone + 'static,
    {
        let dirty = Rc::clone(&self.dirty);
        let cell = self.cell.clone();
        subscribe_signal(signal, move |_| {
            if !dirty.replace(true)
                && let Some(cell) = cell.upgrade()
            {
                cell.mark_layout();
            }
        })
    }
}

impl HydroState {
    /// Reads `signal` at measure time: a plain `snapshot()`, plus — while a
    /// `BuiltSubview` layout pass is collecting — a record in the pass's
    /// dependency set so an update invalidates the cached layout that read
    /// it. Measure paths hold `&mut HydroState`, not the renderer, so layout
    /// reads through `snapshot()` here would otherwise leave no watch
    /// anywhere once the layout result is cached.
    pub(crate) fn measure_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        if let Some(dependencies) = &mut self.layout_dependencies {
            dependencies.watch(signal);
        }
        signal.snapshot()
    }
}

struct SubscribedSnapshotState<T> {
    pending: Vec<nami::watcher::Context<T>>,
    handler: Option<SignalUpdateHandler<T>>,
}

pub(super) struct SubscribedSnapshot<T, G> {
    guard: G,
    state: Rc<RefCell<SubscribedSnapshotState<T>>>,
}

impl<T: 'static, G> SubscribedSnapshot<T, G> {
    pub(super) fn new<S>(signal: &S) -> (Self, T)
    where
        S: Signal<Output = T, Guard = G>,
    {
        let state = Rc::new(RefCell::new(SubscribedSnapshotState {
            pending: Vec::new(),
            handler: None,
        }));
        let guard = signal.watch({
            let state = Rc::clone(&state);
            move |update| {
                let handler = state.borrow().handler.clone();
                if let Some(handler) = handler {
                    handler(update);
                } else {
                    state.borrow_mut().pending.push(update);
                }
            }
        });
        let snapshot = signal.snapshot();
        (Self { guard, state }, snapshot)
    }

    pub(super) fn activate(self, handler: impl Fn(nami::watcher::Context<T>) + 'static) -> G {
        let handler: SignalUpdateHandler<T> = Rc::new(handler);
        let pending = {
            let mut state = self.state.borrow_mut();
            state.handler = Some(Rc::clone(&handler));
            core::mem::take(&mut state.pending)
        };
        for update in pending {
            handler(update);
        }
        self.guard
    }
}

impl SemanticCore {
    /// Pushes a watcher guard onto the store the current phase reads into:
    /// the reading node's `subscriptions` while it records, its
    /// `layout_subscriptions` while it measures/lays out, or the renderer's
    /// outside-read store when no node is reading.
    pub(crate) fn push_guard(&mut self, guard: Retain) {
        match &self.reader {
            Some(reader) => reader.store.borrow_mut().push(guard),
            None => self.outside_frame_retains.push(guard),
        }
    }

    /// The cell an outside-reader watch marks, shared by the identity-less
    /// and identity-keyed paths below.
    fn root_weak(&self) -> Weak<NodeCell> {
        Rc::downgrade(&self.root)
    }

    pub(super) fn watch_signal<S>(&mut self, signal: &S)
    where
        S: Signal + Clone + 'static,
    {
        // Inside a `BuiltSubview` layout pass the read is also a dependency of
        // the cached layout it produces: the dependency set keeps the signal
        // watched — and the sub-view's layout invalidated — for as long as
        // that layout stays cached.
        if let Some(dependencies) = &mut self.state.layout_dependencies {
            dependencies.watch(signal);
        }
        // A reactive *value* change marks the node that read it — `PAINT`
        // while it records, `mark_layout()` while it measures or lays out —
        // and the mark wakes the pump (the spec's commit-1 bridge: any mark
        // still runs the full flush). Reads made with no node reading mark
        // the root cell.
        // Some signals emit synchronously while `watch` registers — a
        // collection's populated emission reports the current contents as an
        // insertion. That registration-time emission echoes the value the
        // caller's `snapshot()` reads this frame, so it must not count as a new
        // update; only a callback invoked after `watch` returns may mark.
        let armed = Rc::new(Cell::new(false));
        let armed_for_watch = Rc::clone(&armed);
        if let Some(reader) = &self.reader {
            // Owner-attributed subscription: the guard lands in the store the
            // current phase selected (`subscriptions` while recording,
            // `layout_subscriptions` while measuring) and is replaced with it.
            let cell = Rc::downgrade(&reader.cell);
            let mark_layout = reader.phase == ReaderPhase::Layout;
            let guard = signal.watch(move |_| {
                if !armed_for_watch.get() {
                    return;
                }
                let Some(cell) = cell.upgrade() else {
                    return;
                };
                if mark_layout {
                    cell.mark_layout();
                } else {
                    cell.mark(Dirty::PAINT);
                }
            });
            armed.set(true);
            reader.store.borrow_mut().push(Retain::new(guard));
            return;
        }
        let Some(identity) = signal.identity() else {
            // Identity-less signal read with no owner: subscribe fresh each
            // flush, retained until the next one.
            let root = self.root_weak();
            let guard = signal.watch(move |_| {
                if armed_for_watch.get()
                    && let Some(root) = root.upgrade()
                {
                    root.mark_layout();
                }
            });
            armed.set(true);
            self.outside_frame_retains.push(Retain::new(guard));
            return;
        };
        // Outside-reader read of an identity-stable signal: one subscription
        // per signal for the renderer's lifetime, reused across flushes. The
        // clone pins the identity allocation, so its address cannot resurface
        // as a different signal under the same key.
        let key = identity.raw();
        let signal_type = core::any::TypeId::of::<S>();
        if let Some(existing) = self.outside_watches.get_mut(&key) {
            assert!(
                existing.signal_type == signal_type,
                "hydrolysis renderer: signal identity {key:#x} is shared by two different signal types (derived-identity hash collision)"
            );
            existing.last_seen = self.outside_watch_generation;
            return;
        }
        let root = self.root_weak();
        let guard = signal.watch(move |_| {
            if armed_for_watch.get()
                && let Some(root) = root.upgrade()
            {
                root.mark_layout();
            }
        });
        armed.set(true);
        self.outside_watches.insert(
            key,
            OutsideWatch {
                _signal: Box::new(signal.clone()),
                signal_type,
                _guard: Retain::new(guard),
                last_seen: self.outside_watch_generation,
            },
        );
    }

    /// Opens a flush for the outside-reader stores: the per-frame guards
    /// from the last flush are consumed, and the watch generation advances
    /// so entries the coming flush does not re-read go stale.
    pub(crate) fn begin_outside_read_frame(&mut self) {
        self.outside_frame_retains.clear();
        self.outside_watch_generation = self
            .outside_watch_generation
            .checked_add(1)
            .expect("hydrolysis renderer: outside watch generation overflow");
    }

    /// Closes a flush for the outside-reader watches: an entry the flush
    /// did not re-read is dropped, along with its signal clone.
    pub(crate) fn finish_outside_read_frame(&mut self) {
        let generation = self.outside_watch_generation;
        self.outside_watches
            .retain(|_, entry| entry.last_seen == generation);
    }

    /// Subscribes `signal` so every update marks the root cell for a
    /// re-layout — the same mark `watch_signal` puts on a signal read
    /// outside any node — and answers the [`Retain`] the caller keeps for
    /// as long as the signal must be observed.
    ///
    /// `watch_signal` ties each subscription to the store the read
    /// attributed it to — the reading node's guard slots, or the
    /// outside-read sweep — so a binding that must stay subscribed for as
    /// long as its watcher lives — the window declaration's own signals,
    /// which the runner observes for the window's whole lifetime — takes
    /// this path and holds the guard itself.
    pub(crate) fn refresh_watch<S>(&self, signal: &S) -> Retain
    where
        S: Signal + Clone + 'static,
    {
        // A signal read outside any node marks `mark_layout()` on the root
        // cell (§B.1): the mark is what `is_settled` and the pump poll for
        // pending work — `mark_layout` carries the `request_refresh` wake.
        let root = self.root_weak();
        subscribe_signal(signal, move |_| {
            if let Some(root) = root.upgrade() {
                root.mark_layout();
            }
        })
    }

    pub(crate) fn read_signal<S>(&mut self, signal: &S) -> S::Output
    where
        S: Signal + Clone + 'static,
    {
        self.watch_signal(signal);
        signal.snapshot()
    }

    pub(crate) fn read_resolved_text_styled(
        &mut self,
        text: &Text,
        env: &Environment,
    ) -> StyledStr {
        let resolved = text.resolve(env);
        self.read_signal(&resolved.content)
    }

    pub(crate) fn set_frame_instant(&mut self, at: Instant) {
        self.frame_instant = at;
        self.signals.set_frame_clock(at);
    }

    pub(crate) const fn frame_instant(&self) -> Instant {
        self.frame_instant
    }

    /// Resolves a boolean toggle signal into its animated progress and the
    /// current target value (the direction the animation is heading).
    pub(crate) fn resolve_toggle_progress<S>(
        &mut self,
        signal: &S,
        default_animation: Animation,
    ) -> (f32, bool)
    where
        S: Signal<Output = bool> + Clone + 'static,
    {
        let Some(identity) = signal.identity() else {
            let selected = self.read_signal(signal);
            return (if selected { 1.0 } else { 0.0 }, selected);
        };
        let (subscription, selected) = SubscribedSnapshot::new(signal);
        let now = self.frame_instant;
        let target = if selected { 1.0 } else { 0.0 };
        let key = AnimationKey::scalar(identity);
        let handle = self.animation_controller.bind_scalar_target(
            key,
            target,
            default_animation.clone(),
            now,
        );
        let watcher_handle = handle.clone();
        let owner = self.mark_owner_for_animation(key, Dirty::PAINT);
        let clock = self.signals.clone();
        // A subscription's registration emission echoes the value `bind`
        // already sampled — only a later fire may mark the owner.
        let armed = Rc::new(Cell::new(false));
        let armed_for_watch = Rc::clone(&armed);
        let guard = subscription.activate(move |update| {
            let target = if *update.value() { 1.0 } else { 0.0 };
            let animation = update
                .metadata()
                .try_get::<Animation>()
                .unwrap_or_else(|| default_animation.clone());
            watcher_handle.apply_target(target, Some(animation), clock.frame_clock());
            if armed_for_watch.get()
                && let Some(cell) = owner.upgrade()
            {
                cell.mark(Dirty::PAINT);
            }
        });
        armed.set(true);
        self.push_guard(Retain::new(guard));
        (handle.sample(now).clamp(0.0, 1.0), selected)
    }

    /// Registers `key`'s animation slot against the node now reading and
    /// returns that node's `Weak` for the watcher's own marks. With no node
    /// reading, the root cell stands in as the owner.
    pub(crate) fn mark_owner_for_animation(
        &mut self,
        key: AnimationKey,
        dirty: Dirty,
    ) -> Weak<NodeCell> {
        let cell = match &self.reader {
            Some(reader) => Rc::clone(&reader.cell),
            None => Rc::clone(&self.root),
        };
        self.bind_animation_owner(key, &cell, dirty);
        Rc::downgrade(&cell)
    }

    pub(crate) fn sample_widget_scalar_target(
        &mut self,
        key: AnimationKey,
        target: f32,
        animation: Animation,
    ) -> f32 {
        let now = self.frame_instant;
        self.animation_controller
            .bind_scalar_target(key, target, animation, now)
            .sample(now)
    }

    pub(crate) fn sample_radio_indicator_state(
        &mut self,
        key: AnimationKey,
        selected: bool,
        motion: &RadioSelectionMotion,
    ) -> RadioIndicatorState {
        self.animation_controller
            .bind_radio_indicator(key, selected, motion, self.frame_instant)
    }

    /// Sample a free-running repeating phase (e.g. an indeterminate progress
    /// indicator). `node_id` is the stable identity of the owning node (its retained
    /// `Rc` address), so the phase slot keys off node identity and survives across
    /// frames and structural changes — unlike a positional `render_depth`, which
    /// shifts when a sibling subtree's node count changes and would reset the phase.
    pub(crate) fn sample_repeating_motion(&mut self, cycle: Duration, node_id: usize) -> Duration {
        let key = AnimationKey::renderer_local_repeating(node_id);
        self.animation_controller
            .bind_repeating_phase(key, cycle, self.frame_instant)
    }

    pub fn advance_animations(&mut self) -> bool {
        let now = self.frame_instant;
        let pending_releases = self.flush_interaction_releases(now);
        let ticked = self.animation_controller.tick(now);
        if ticked {
            // Mark every live owner — a slot finishing this tick still gets
            // its last sample marked — then keep only the entries whose
            // slot is still animating, so an idle animation stops marking.
            let active = self.animation_controller.active_scalar_keys();
            self.animation_owners.retain(|key, owner| {
                let Some(cell) = owner.cell.upgrade() else {
                    return false;
                };
                // The tick's own scheduling is the `Animate` arm — marking
                // through `mark()` would read the animation's continuation
                // as an unapplied change every frame.
                cell.mark_quiet(owner.dirty);
                active.contains(key)
            });
        }
        ticked
            || pending_releases
            || self.navigation.slots.values().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
                    || slot
                        .interactive_pop
                        .as_ref()
                        .is_some_and(NavigationInteractivePop::is_animating)
            })
    }

    pub fn animations_active(&self) -> bool {
        let now = self.frame_instant;
        self.animation_controller.has_active(now)
            || self.has_pending_interaction_releases(now)
            || self.navigation.slots.values().any(|slot| {
                slot.transition
                    .as_ref()
                    .is_some_and(|state| state.is_active(now))
                    || slot
                        .interactive_pop
                        .as_ref()
                        .is_some_and(NavigationInteractivePop::is_animating)
            })
    }
}
