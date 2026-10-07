//! Reactive inputs: signal watching and animated-value sampling that bind
//! `WaterUI` signals to frame triggers and the animation controller.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

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
    /// Refresh requester the dependency subscriptions' update callbacks use.
    signals: FrameSignals,
}

impl LayoutDependencies {
    /// An empty set for a sub-view whose layout has not run yet.
    pub(super) fn new(signals: FrameSignals) -> Self {
        Self {
            identities: SignalWatchRegistry::default(),
            anonymous: Vec::new(),
            dirty: Rc::new(Cell::new(false)),
            signals,
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

    /// One subscription marking the set dirty on an update. A refresh is
    /// requested only on the dirty edge — the owning `BuiltSubview` folds the
    /// flag into `needs_layout` on that frame, so a busy signal read by a
    /// sub-view whose layout stays cached (hidden off-screen, or yet to
    /// settle) requests one frame, not one per write.
    fn subscribe<S>(&self, signal: &S) -> Retain
    where
        S: Signal + Clone + 'static,
    {
        let dirty = Rc::clone(&self.dirty);
        let signals = self.signals.clone();
        subscribe_signal(signal, move |_| {
            if !dirty.replace(true) {
                signals.request_refresh();
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
        // A reactive *value* change re-flushes the retained tree (re-read, re-layout,
        // re-encode) — the cheap per-frame pump — instead of re-running the whole view
        // `body()`. Structural changes go through `Dynamic`/`when` (a patch), not a
        // plain signal read, so a refresh is sufficient here.
        let Some(identity) = signal.identity() else {
            // Identity-less signal: subscribe fresh each read, retained for one frame.
            let guard = self.refresh_watch(signal);
            self.lifecycle.current_frame_retain.push(guard);
            return;
        };
        // Identity-stable signal: one subscription per signal, reused across frames
        // for as long as the flush keeps reading it (see `SignalWatchRegistry`).
        let key = identity.raw();
        let signal_type = core::any::TypeId::of::<S>();
        if self.lifecycle.signal_watches.mark_seen(key, signal_type) {
            return;
        }
        let guard = self.refresh_watch(signal);
        self.lifecycle
            .signal_watches
            .insert(key, signal_type, Box::new(signal.clone()), guard);
    }

    /// Subscribes `signal` so every update requests a refresh through this
    /// core's frame signals — the same wake `watch_signal`'s subscriptions
    /// carry — and answers the [`Retain`] the caller keeps for as long as
    /// the signal must be observed.
    ///
    /// `watch_signal`'s subscriptions live for the frame that registers
    /// them; a binding that must stay subscribed across idle frames — the
    /// window declaration's own signals, which the runner watches for the
    /// window's whole lifetime — cannot reach a frame to re-register from,
    /// so it takes this path and holds the guard itself.
    pub(crate) fn refresh_watch<S>(&self, signal: &S) -> Retain
    where
        S: Signal + Clone + 'static,
    {
        let signals = self.signals.clone();
        subscribe_signal(signal, move |_| signals.request_refresh())
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
        let signals = self.signals.clone();
        let guard = subscription.activate(move |update| {
            let target = if *update.value() { 1.0 } else { 0.0 };
            let animation = update
                .metadata()
                .try_get::<Animation>()
                .unwrap_or_else(|| default_animation.clone());
            watcher_handle.apply_target(target, Some(animation), signals.frame_clock());
            signals.request_refresh();
        });
        self.lifecycle.current_frame_retain.push(Retain::new(guard));
        (handle.sample(now).clamp(0.0, 1.0), selected)
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
        self.animation_controller.tick(now)
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
