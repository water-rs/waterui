// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;
use std::collections::BTreeSet;
use waterui::navigation::{
    AnyNavigationTransition, NavigationDestinationState, NavigationTransactionId,
    NavigationTransitionDirection,
};
use waterui_backend_core::widget::NavigationMotion;
use waterui_core::id::Id;

pub const ROOT_NAVIGATION_IDENTITY: u64 = 0;

#[derive(Clone)]
pub struct NavigationMatchedElement {
    pub(crate) bounds: kurbo::Rect,
    pub layers: CapturedLayers,
}

#[derive(Clone, Default)]
pub struct NavigationCapturedScene {
    /// The page, recorded in its own space with every layer it presents.
    pub(crate) layers: CapturedLayers,
    pub(crate) sources: BTreeMap<Id, NavigationMatchedElement>,
    pub(crate) destinations: BTreeMap<Id, NavigationMatchedElement>,
    /// The leading bar reserve the page was recorded under. A scene captured
    /// before a compact split injected the reserve replays a title painted in
    /// the chevron's space, so cache lookups reject a reserve mismatch.
    pub(crate) leading_reserve: f64,
}

impl NavigationCapturedScene {
    pub(crate) fn composed(&self) -> CapturedLayers {
        let mut layers = self.layers.clone();
        for element in self.sources.values().chain(self.destinations.values()) {
            layers.extend(&element.layers);
        }
        layers
    }

    pub(crate) fn composed_without(&self, source: bool, id: Id) -> CapturedLayers {
        let mut layers = self.layers.clone();
        for (element_id, element) in &self.sources {
            if !source || *element_id != id {
                layers.extend(&element.layers);
            }
        }
        for (element_id, element) in &self.destinations {
            if source || *element_id != id {
                layers.extend(&element.layers);
            }
        }
        layers
    }
}

#[derive(Default)]
pub struct NavigationSceneCapture {
    sources: BTreeMap<Id, NavigationMatchedElement>,
    destinations: BTreeMap<Id, NavigationMatchedElement>,
    capturing_element: bool,
}

pub struct HydroNavigationEntry {
    pub(crate) identity: u64,
    pub(crate) content: RetainedSubview,
    pub(crate) state: NavigationDestinationState,
    /// What this destination asked to arrive with, if it asked for anything.
    ///
    /// A matched transition names a pair that differs per destination, so the
    /// stack cannot name it once for all of them; the entry carries what its
    /// own destination declared, and the stack's style is the fallback.
    pub(crate) transition: Option<AnyNavigationTransition>,
}

impl HydroNavigationEntry {
    fn new(identity: u64, view: NavigationView) -> Self {
        let NavigationView {
            bar,
            content,
            state,
            transition,
        } = view;
        Self {
            identity,
            content: RetainedSubview::new(AnyView::new(NavigationView {
                bar,
                content,
                state: NavigationDestinationState::default(),
                transition: None,
            })),
            state,
            transition,
        }
    }
}

pub struct HydroNavigationEvent {
    pub(crate) transaction_id: NavigationTransactionId,
    pub(crate) previous_identity: u64,
    pub(crate) current_identity: u64,
    pub removed: Vec<HydroNavigationEntry>,
}

pub type NavigationEntries = Rc<RefCell<Vec<HydroNavigationEntry>>>;
pub type NavigationEvents = Rc<RefCell<Vec<HydroNavigationEvent>>>;

#[derive(Default)]
pub struct NavigationState {
    pub slots: BTreeMap<NavigationKey, NavigationSlot>,
    /// Addresses bound this frame. Held as plain addresses rather than keys so
    /// the slot map holds the only strong lease on each retained stack, which
    /// is what [`NavigationKey::is_retained_elsewhere`] measures.
    active: BTreeSet<usize>,
}

/// Stable identity of one retained navigation stack.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct NavigationKey(RetainedIdentity);

impl NavigationKey {
    pub(crate) fn for_rc<T: 'static>(owner: &Rc<T>) -> Self {
        Self(RetainedIdentity::for_rc(owner))
    }

    pub(crate) const fn address(&self) -> usize {
        self.0.address()
    }

    /// Whether the retained stack behind this key is still held by the render
    /// tree.
    pub(crate) fn is_retained_elsewhere(&self) -> bool {
        self.0.is_retained_elsewhere()
    }
}

pub struct NavigationSlot {
    pub entries: NavigationEntries,
    pub(crate) events: NavigationEvents,
    pub(crate) controller: NavigationController,
    pub(crate) last_depth: usize,
    pub(crate) active_identity: u64,
    pub(crate) root_state: Option<NavigationDestinationState>,
    pub(crate) root_is_active: bool,
    pub(crate) last_scene: Option<NavigationCapturedScene>,
    pub(crate) scene_cache: BTreeMap<u64, NavigationCapturedScene>,
    pub(crate) transition: Option<NavigationTransitionState>,
    /// Entries that left the stack this transaction. Held behind the same
    /// `Rc<RefCell>` lease as `entries` because a departing page stays
    /// renderable until its transition completes — a stale cached scene is
    /// re-recorded from it even though it is no longer in `entries`.
    pub(crate) pending_removed: NavigationEntries,
    pub(crate) pending_appearance: bool,
    pub(crate) pending_transaction_id: Option<NavigationTransactionId>,
    pub(crate) skip_next_pop_transition: bool,
    pub(crate) interactive_pop: Option<NavigationInteractivePop>,
}

#[derive(Clone)]
pub struct NavigationTransitionState {
    pub style: AnyNavigationTransition,
    pub(crate) direction: NavigationTransitionDirection,
    pub(crate) from_scene: NavigationCapturedScene,
    pub(crate) to_scene: NavigationCapturedScene,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

pub struct HydroNavigationController {
    pub entries: NavigationEntries,
    pub(crate) events: NavigationEvents,
    pub(crate) next_entry_identity: u64,
    pub(crate) signals: FrameSignals,
}

#[derive(Clone, Copy)]
pub enum NavigationInteractivePopPhase {
    Dragging,
    Completing {
        started_at: Instant,
        initial_progress: f64,
    },
    Cancelling {
        started_at: Instant,
        initial_progress: f64,
    },
}

/// How an interactive pop leaves the dragging phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationInteractivePopOutcome {
    /// Commit when progress is at least one half; otherwise cancel.
    ByProgress,
    /// Commit whatever the current progress.
    Commit,
    /// Cancel whatever the current progress.
    Cancel,
}

/// One navigation stack that can receive system back for this frame.
///
/// Registered while the stack renders at depth greater than zero, in the
/// same place as the edge-drag target. The last registration of the frame is
/// the frontmost stack.
#[derive(Clone)]
pub struct NavigationBackTarget {
    pub(crate) slot_key: NavigationKey,
    pub(crate) width: f64,
    pub(crate) from_scene: NavigationCapturedScene,
    pub(crate) to_scene: NavigationCapturedScene,
    pub(crate) controller: NavigationController,
}

/// The system-back gesture currently in flight.
///
/// Kept across frames, like the active pointer drag: the per-frame back-target
/// list is rebuilt, but a gesture that started against the previous frame's
/// frontmost target continues until that gesture ends.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SystemBackGesture {
    /// No system-back gesture is in flight.
    #[default]
    Idle,
    /// The gesture's `Started` was refused. Later phases of this gesture do
    /// nothing until the next `Started`.
    Ignored,
    /// A predictive pop is in flight on this stack.
    Active(NavigationKey),
}

pub struct NavigationInteractivePop {
    pub(crate) start_x: f64,
    pub width: f64,
    pub(crate) progress: f64,
    pub(crate) phase: NavigationInteractivePopPhase,
    pub(crate) from_scene: NavigationCapturedScene,
    pub(crate) to_scene: NavigationCapturedScene,
}

impl NavigationState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.active.clear();
    }

    /// Drops the slots whose retained stack the render tree no longer holds.
    ///
    /// A stack keeps its slot — its pushed path, destination lifecycle state,
    /// and cached scenes — while it exists but is not being drawn. Tabs render
    /// only the selected tab, so pruning by "bound this frame" instead would
    /// destroy every background tab's navigation state and leave the retained
    /// stack unable to reinstall its root on return.
    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots
            .retain(|key, _| self.active.contains(&key.address()) || key.is_retained_elsewhere());
    }
}

impl NavigationSlot {
    pub(crate) fn new(signals: FrameSignals) -> Self {
        let entries = Rc::new(RefCell::new(Vec::new()));
        let events = Rc::new(RefCell::new(Vec::new()));
        let controller = NavigationController::new(HydroNavigationController {
            entries: Rc::clone(&entries),
            events: Rc::clone(&events),
            next_entry_identity: 1,
            signals,
        });

        Self {
            entries,
            events,
            controller,
            last_depth: 0,
            active_identity: ROOT_NAVIGATION_IDENTITY,
            root_state: None,
            root_is_active: false,
            last_scene: None,
            scene_cache: BTreeMap::new(),
            transition: None,
            pending_removed: Rc::new(RefCell::new(Vec::new())),
            pending_appearance: false,
            pending_transaction_id: None,
            skip_next_pop_transition: false,
            interactive_pop: None,
        }
    }
}

impl NavigationInteractivePop {
    pub(crate) fn new(
        start_x: f64,
        width: f64,
        from_scene: NavigationCapturedScene,
        to_scene: NavigationCapturedScene,
    ) -> Self {
        assert!(width > 0.0, "interactive navigation width must be positive");
        Self {
            start_x,
            width,
            progress: 0.0,
            phase: NavigationInteractivePopPhase::Dragging,
            from_scene,
            to_scene,
        }
    }

    pub(crate) fn update(&mut self, x: f64) -> bool {
        if !matches!(self.phase, NavigationInteractivePopPhase::Dragging) {
            return false;
        }
        let progress = ((x - self.start_x) / self.width).clamp(0.0, 1.0);
        if (progress - self.progress).abs() <= f64::EPSILON {
            return false;
        }
        self.progress = progress;
        true
    }

    /// Sets the dragging progress directly, as a platform back gesture reports it.
    ///
    /// `progress` is clamped into `0..=1`. A non-finite value is a contract
    /// breach. Returns whether the stored progress changed. A pop that has
    /// already left the dragging phase ignores the report.
    pub(crate) fn set_progress(&mut self, progress: f64) -> bool {
        assert!(
            progress.is_finite(),
            "interactive navigation progress must be finite"
        );
        if !matches!(self.phase, NavigationInteractivePopPhase::Dragging) {
            return false;
        }
        let progress = progress.clamp(0.0, 1.0);
        if (progress - self.progress).abs() <= f64::EPSILON {
            return false;
        }
        self.progress = progress;
        true
    }

    pub(crate) fn finish(&mut self, now: Instant, outcome: NavigationInteractivePopOutcome) {
        let cancel = match outcome {
            NavigationInteractivePopOutcome::Cancel => true,
            NavigationInteractivePopOutcome::Commit => false,
            NavigationInteractivePopOutcome::ByProgress => self.progress < 0.5,
        };
        self.phase = if cancel {
            NavigationInteractivePopPhase::Cancelling {
                started_at: now,
                initial_progress: self.progress,
            }
        } else {
            NavigationInteractivePopPhase::Completing {
                started_at: now,
                initial_progress: self.progress,
            }
        };
    }

    pub(crate) fn sample(&mut self, now: Instant, motion: NavigationMotion) -> (f64, bool, bool) {
        match self.phase {
            NavigationInteractivePopPhase::Dragging => (self.progress, false, false),
            NavigationInteractivePopPhase::Completing {
                started_at,
                initial_progress,
            } => {
                let remaining = 1.0 - initial_progress;
                let elapsed = now.saturating_duration_since(started_at).as_secs_f64();
                let span = (motion.transition_duration.as_secs_f64() * remaining).max(f64::EPSILON);
                let cycle = (elapsed / span).clamp(0.0, 1.0);
                self.progress =
                    f64::mul_add(remaining, eased_progress(cycle, motion), initial_progress);
                (self.progress, self.progress >= 1.0, false)
            }
            NavigationInteractivePopPhase::Cancelling {
                started_at,
                initial_progress,
            } => {
                let elapsed = now.saturating_duration_since(started_at).as_secs_f64();
                let span =
                    (motion.transition_duration.as_secs_f64() * initial_progress).max(f64::EPSILON);
                let cycle = (elapsed / span).clamp(0.0, 1.0);
                self.progress = initial_progress * (1.0 - eased_progress(cycle, motion));
                (self.progress, false, self.progress <= 0.0)
            }
        }
    }

    pub(crate) const fn is_animating(&self) -> bool {
        !matches!(self.phase, NavigationInteractivePopPhase::Dragging)
    }
}

impl NavigationTransitionState {
    pub(crate) const fn new(
        style: AnyNavigationTransition,
        direction: NavigationTransitionDirection,
        from_scene: NavigationCapturedScene,
        to_scene: NavigationCapturedScene,
        started_at: Instant,
        duration: Duration,
    ) -> Self {
        Self {
            style,
            direction,
            from_scene,
            to_scene,
            started_at,
            duration,
        }
    }

    pub(crate) fn progress(&self, now: Instant) -> f64 {
        let elapsed = now.saturating_duration_since(self.started_at);
        (elapsed.as_secs_f64() / self.duration.as_secs_f64()).clamp(0.0, 1.0)
    }

    pub(crate) fn eased_progress(&self, now: Instant, motion: NavigationMotion) -> f64 {
        eased_progress(self.progress(now), motion)
    }

    pub(crate) fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }
}

#[allow(clippy::cast_possible_truncation)]
fn eased_progress(progress: f64, motion: NavigationMotion) -> f64 {
    f64::from(
        motion
            .transition_easing
            .ease(progress as f32)
            .clamp(0.0, 1.0),
    )
}

impl CustomNavigationController for HydroNavigationController {
    fn apply(&mut self, transaction: NavigationTransaction) {
        let NavigationTransaction {
            id: transaction_id,
            retained_prefix,
            removed,
            inserted,
        } = transaction;
        let mut entries = self.entries.borrow_mut();
        assert_eq!(
            retained_prefix + removed,
            entries.len(),
            "Hydrolysis navigation transaction must replace the current suffix"
        );
        let previous_identity = entries
            .last()
            .map_or(ROOT_NAVIGATION_IDENTITY, |entry| entry.identity);
        let removed = entries.drain(retained_prefix..).collect();
        entries.extend(inserted.into_iter().map(|builder| {
            let identity = self.next_entry_identity;
            self.next_entry_identity = self
                .next_entry_identity
                .checked_add(1)
                .expect("Hydrolysis navigation entry identity overflowed");
            HydroNavigationEntry::new(identity, builder.build())
        }));
        let current_identity = entries
            .last()
            .map_or(ROOT_NAVIGATION_IDENTITY, |entry| entry.identity);
        drop(entries);
        self.events.borrow_mut().push(HydroNavigationEvent {
            transaction_id,
            previous_identity,
            current_identity,
            removed,
        });
        self.signals.request_refresh();
    }
}

fn take_destination_state(
    slot: &mut NavigationSlot,
    identity: u64,
) -> Option<NavigationDestinationState> {
    if identity == ROOT_NAVIGATION_IDENTITY {
        return slot.root_state.take();
    }
    if let Some(entry) = slot
        .entries
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.identity == identity)
    {
        return Some(core::mem::take(&mut entry.state));
    }
    if let Some(entry) = slot
        .pending_removed
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.identity == identity)
    {
        return Some(core::mem::take(&mut entry.state));
    }
    for event in slot.events.borrow_mut().iter_mut() {
        if let Some(entry) = event
            .removed
            .iter_mut()
            .find(|entry| entry.identity == identity)
        {
            return Some(core::mem::take(&mut entry.state));
        }
    }
    panic!("Hydrolysis navigation destination identity {identity} is not retained");
}

fn put_destination_state(
    slot: &mut NavigationSlot,
    identity: u64,
    state: NavigationDestinationState,
) {
    if identity == ROOT_NAVIGATION_IDENTITY {
        assert!(
            slot.root_state.is_none(),
            "Hydrolysis navigation root state was returned twice"
        );
        slot.root_state = Some(state);
        return;
    }
    if let Some(entry) = slot
        .entries
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.identity == identity)
    {
        entry.state = state;
        return;
    }
    if let Some(entry) = slot
        .pending_removed
        .borrow_mut()
        .iter_mut()
        .find(|entry| entry.identity == identity)
    {
        entry.state = state;
        return;
    }
    for event in slot.events.borrow_mut().iter_mut() {
        if let Some(entry) = event
            .removed
            .iter_mut()
            .find(|entry| entry.identity == identity)
        {
            entry.state = state;
            return;
        }
    }
    panic!("Hydrolysis navigation destination identity {identity} was lost during its callback");
}

impl SemanticCore {
    pub(crate) fn install_navigation_root_state(
        &mut self,
        slot_key: &NavigationKey,
        state: NavigationDestinationState,
    ) {
        let slot = self
            .navigation
            .slots
            .get_mut(slot_key)
            .expect("Hydrolysis navigation slot missing during root installation");
        assert!(
            slot.root_state.is_none(),
            "Hydrolysis navigation root state was installed more than once"
        );
        slot.root_state = Some(state);
    }

    pub(crate) fn activate_navigation_root_if_needed(
        &mut self,
        slot_key: &NavigationKey,
        env: &Environment,
    ) {
        let should_activate = {
            let slot = self
                .navigation
                .slots
                .get(slot_key)
                .expect("Hydrolysis navigation slot missing during root activation");
            slot.active_identity == ROOT_NAVIGATION_IDENTITY
                && slot.pending_transaction_id.is_none()
                && !slot.root_is_active
        };
        if !should_activate {
            return;
        }
        let slot = self
            .navigation
            .slots
            .get_mut(slot_key)
            .expect("Hydrolysis navigation slot missing during root activation");
        let mut state = slot
            .root_state
            .take()
            .expect("Hydrolysis navigation root state is not installed");
        state.appeared(env);
        slot.root_state = Some(state);
        slot.root_is_active = true;
    }

    pub(crate) fn bind_navigation_entries(&mut self, key: &NavigationKey) -> NavigationEntries {
        self.navigation.active.insert(key.address());
        let slot = self
            .navigation
            .slots
            .entry(key.clone())
            .or_insert_with(|| NavigationSlot::new(self.signals.clone()));
        Rc::clone(&slot.entries)
    }

    pub(crate) fn finish_interactive_navigation_pop(
        &mut self,
        outcome: NavigationInteractivePopOutcome,
    ) -> bool {
        let now = self.frame_instant();
        // A system-back gesture owns its stack's pop until the platform says
        // the gesture ended. A pointer release must not apply the drag
        // threshold to that pop.
        let system_back_slot = match &self.hit_test.system_back {
            SystemBackGesture::Active(key) => Some(key.clone()),
            SystemBackGesture::Idle | SystemBackGesture::Ignored => None,
        };
        let mut changed = false;
        for (key, slot) in &mut self.navigation.slots {
            if system_back_slot.as_ref() == Some(key) {
                continue;
            }
            if let Some(interactive) = slot.interactive_pop.as_mut()
                && matches!(interactive.phase, NavigationInteractivePopPhase::Dragging)
            {
                interactive.finish(now, outcome);
                changed = true;
            }
        }
        if changed {
            self.signals.request_refresh();
        }
        changed
    }

    /// Records one stack as a system-back target for the frame being built.
    ///
    /// The last target registered this frame is the frontmost one.
    pub(crate) fn register_back_target(&mut self, target: NavigationBackTarget) {
        self.hit_test.back_targets.push(target);
    }

    /// Whether the frame that just rendered registered any system-back target.
    #[cfg_attr(
        not(any(target_os = "android", test)),
        expect(
            dead_code,
            reason = "the Android runner reports this after each frame, and the navigation-back tests read it"
        )
    )]
    #[must_use]
    pub(crate) const fn has_back_navigation_target(&self) -> bool {
        !self.hit_test.back_targets.is_empty()
    }

    /// Drives one system-back phase against the frontmost target.
    ///
    /// `Started` runs the pop-attempt policy. A refusal ignores the rest of
    /// that gesture. An allowed gesture begins an interactive pop the same
    /// way an edge drag does. `Invoked` with no preceding `Started` runs the
    /// policy and, when it allows the pop, requests the ordinary animated pop.
    pub(crate) fn handle_back_navigation(
        &mut self,
        event: crate::BackNavigation,
        env: &Environment,
    ) -> bool {
        match event {
            crate::BackNavigation::Started { .. } => self.begin_system_back(env),
            crate::BackNavigation::Progressed { progress } => self.progress_system_back(progress),
            crate::BackNavigation::Cancelled => {
                self.end_system_back(NavigationInteractivePopOutcome::Cancel)
            }
            crate::BackNavigation::Invoked => self.invoke_system_back(env),
        }
    }

    fn begin_system_back(&mut self, env: &Environment) -> bool {
        // A new gesture replaces one that was still running.
        if let SystemBackGesture::Active(key) = self.hit_test.system_back.clone() {
            if let Some(slot) = self.navigation.slots.get_mut(&key) {
                slot.interactive_pop = None;
            }
            self.hit_test.system_back = SystemBackGesture::Idle;
        }
        let Some(target) = self.hit_test.back_targets.last().cloned() else {
            self.hit_test.system_back = SystemBackGesture::Ignored;
            return false;
        };
        if !self.attempt_navigation_pop(&target.slot_key, env) {
            self.hit_test.system_back = SystemBackGesture::Ignored;
            return false;
        }
        let slot = self
            .navigation
            .slots
            .get_mut(&target.slot_key)
            .expect("Hydrolysis navigation slot missing");
        slot.transition = None;
        slot.interactive_pop = Some(NavigationInteractivePop::new(
            0.0,
            target.width,
            target.from_scene,
            target.to_scene,
        ));
        self.hit_test.system_back = SystemBackGesture::Active(target.slot_key);
        self.signals.request_refresh();
        true
    }

    fn progress_system_back(&mut self, progress: f64) -> bool {
        let SystemBackGesture::Active(key) = self.hit_test.system_back.clone() else {
            return false;
        };
        let changed = self
            .navigation
            .slots
            .get_mut(&key)
            .and_then(|slot| slot.interactive_pop.as_mut())
            .is_some_and(|pop| pop.set_progress(progress));
        if changed {
            self.signals.request_refresh();
        }
        changed
    }

    fn end_system_back(&mut self, outcome: NavigationInteractivePopOutcome) -> bool {
        match self.hit_test.system_back.clone() {
            SystemBackGesture::Idle => false,
            SystemBackGesture::Ignored => {
                self.hit_test.system_back = SystemBackGesture::Idle;
                false
            }
            SystemBackGesture::Active(key) => {
                self.hit_test.system_back = SystemBackGesture::Idle;
                self.finish_system_back_slot(&key, outcome)
            }
        }
    }

    fn invoke_system_back(&mut self, env: &Environment) -> bool {
        match self.hit_test.system_back.clone() {
            SystemBackGesture::Active(key) => {
                self.hit_test.system_back = SystemBackGesture::Idle;
                self.finish_system_back_slot(&key, NavigationInteractivePopOutcome::Commit)
            }
            SystemBackGesture::Ignored => {
                self.hit_test.system_back = SystemBackGesture::Idle;
                false
            }
            SystemBackGesture::Idle => self.invoke_back_without_gesture(env),
        }
    }

    fn invoke_back_without_gesture(&mut self, env: &Environment) -> bool {
        let Some(target) = self.hit_test.back_targets.last().cloned() else {
            return false;
        };
        if !self.attempt_navigation_pop(&target.slot_key, env) {
            return false;
        }
        target.controller.request_pop(1);
        true
    }

    fn finish_system_back_slot(
        &mut self,
        key: &NavigationKey,
        outcome: NavigationInteractivePopOutcome,
    ) -> bool {
        let now = self.frame_instant();
        let slot = self
            .navigation
            .slots
            .get_mut(key)
            .expect("Hydrolysis navigation slot missing");
        let Some(interactive) = slot.interactive_pop.as_mut() else {
            return false;
        };
        if !matches!(interactive.phase, NavigationInteractivePopPhase::Dragging) {
            return false;
        }
        interactive.finish(now, outcome);
        self.signals.request_refresh();
        true
    }

    pub(crate) fn attempt_navigation_pop(
        &mut self,
        slot_key: &NavigationKey,
        env: &Environment,
    ) -> bool {
        let slot = self
            .navigation
            .slots
            .get_mut(slot_key)
            .expect("Hydrolysis navigation slot missing during pop attempt");
        let identity = slot.active_identity;
        let Some(mut state) = take_destination_state(slot, identity) else {
            return false;
        };
        let allowed = state.attempt_pop(env);
        put_destination_state(slot, identity, state);
        allowed
    }

    /// Applies one batch of pending navigation events — the record a
    /// push/pop/replace left behind when it mutated `entries` — folding the
    /// removals into `pending_removed`, marking the transaction pending, and
    /// firing `disappeared` on the destination that yielded the active
    /// position.
    ///
    /// This is the lifecycle point both pipelines share: the state change
    /// already happened on the retained tree, so the rendered walk and the
    /// semantic emit each call it once. The rendered path then animates the
    /// transition before [`Self::complete_navigation_transaction`] fires
    /// `appeared`/`popped`; the semantic emit completes it in place — there
    /// is no scene to animate.
    ///
    /// `active_identity` is the identity of the entry the mutated `entries`
    /// stack now shows; the batch's final `current_identity` is asserted
    /// against it. Returns the previous active identity and stack depth when
    /// a batch was applied — the rendered walk uses them to animate from the
    /// departed scene.
    pub(crate) fn apply_navigation_events(
        &mut self,
        slot_key: &NavigationKey,
        active_identity: u64,
        env: &Environment,
    ) -> Option<(u64, usize)> {
        let events = {
            let slot = self
                .navigation
                .slots
                .get_mut(slot_key)
                .expect("Hydrolysis navigation slot missing");
            slot.events.borrow_mut().drain(..).collect::<Vec<_>>()
        };
        if events.is_empty() {
            return None;
        }
        for pair in events.windows(2) {
            assert_eq!(
                pair[0].current_identity, pair[1].previous_identity,
                "Hydrolysis navigation events must form one atomic transaction chain"
            );
        }
        let final_identity = events
            .last()
            .expect("non-empty navigation event batch must have a last event")
            .current_identity;
        assert_eq!(
            final_identity, active_identity,
            "Hydrolysis navigation event result must match retained stack entries"
        );
        let final_transaction_id = events
            .last()
            .expect("non-empty navigation event batch must have a last event")
            .transaction_id;
        let (previous_identity, previous_depth) = {
            let slot = self
                .navigation
                .slots
                .get_mut(slot_key)
                .expect("Hydrolysis navigation slot missing");
            assert_eq!(
                events
                    .first()
                    .expect("non-empty navigation event batch must have a first event")
                    .previous_identity,
                slot.active_identity,
                "Hydrolysis navigation event must start from the rendered destination"
            );
            if let Some(previous_transaction_id) = slot.pending_transaction_id.take() {
                let _ = slot
                    .controller
                    .transition_cancelled(previous_transaction_id);
            }
            let previous_identity = slot.active_identity;
            let previous_depth = slot.last_depth;
            for event in events {
                slot.pending_removed.borrow_mut().extend(event.removed);
            }
            slot.transition = None;
            slot.interactive_pop = None;
            slot.pending_transaction_id = Some(final_transaction_id);
            slot.pending_appearance = previous_identity != final_identity;
            slot.active_identity = final_identity;
            (previous_identity, previous_depth)
        };
        if previous_identity != active_identity {
            self.navigation_destination_disappeared(slot_key, previous_identity, env);
        }
        Some((previous_identity, previous_depth))
    }

    pub(crate) fn navigation_destination_disappeared(
        &mut self,
        slot_key: &NavigationKey,
        identity: u64,
        env: &Environment,
    ) {
        let slot = self
            .navigation
            .slots
            .get_mut(slot_key)
            .expect("Hydrolysis navigation slot missing during disappear callback");
        if identity == ROOT_NAVIGATION_IDENTITY {
            if !slot.root_is_active {
                return;
            }
            slot.root_is_active = false;
        }
        if let Some(mut state) = take_destination_state(slot, identity) {
            state.disappeared(env);
            put_destination_state(slot, identity, state);
        }
    }

    pub(crate) fn complete_navigation_transaction(
        &mut self,
        slot_key: &NavigationKey,
        env: &Environment,
    ) {
        let (mut removed, appearance_identity, transaction_id, controller) = {
            let slot = self
                .navigation
                .slots
                .get_mut(slot_key)
                .expect("Hydrolysis navigation slot missing during transaction completion");
            let appearance_identity = slot.pending_appearance.then_some(slot.active_identity);
            slot.pending_appearance = false;
            for entry in slot.pending_removed.borrow().iter() {
                slot.scene_cache.remove(&entry.identity);
            }
            (
                core::mem::take(&mut *slot.pending_removed.borrow_mut()),
                appearance_identity,
                slot.pending_transaction_id.take(),
                slot.controller.clone(),
            )
        };

        for entry in &mut removed {
            entry.state.popped(env);
        }

        if let Some(identity) = appearance_identity {
            let slot = self
                .navigation
                .slots
                .get_mut(slot_key)
                .expect("Hydrolysis navigation slot missing during appear callback");
            if let Some(mut state) = take_destination_state(slot, identity) {
                state.appeared(env);
                put_destination_state(slot, identity, state);
                if identity == ROOT_NAVIGATION_IDENTITY {
                    slot.root_is_active = true;
                }
            }
        }

        if let Some(transaction_id) = transaction_id {
            let _ = controller.transition_completed(transaction_id);
        }
    }
}

impl HydrolysisRenderer {
    pub(crate) fn begin_navigation_scene_capture(&mut self) {
        self.navigation_captures
            .push(NavigationSceneCapture::default());
    }

    pub(crate) fn finish_navigation_scene_capture(
        &mut self,
        layers: CapturedLayers,
    ) -> NavigationCapturedScene {
        let capture = self
            .navigation_captures
            .pop()
            .expect("navigation scene capture must be active");
        assert!(
            !capture.capturing_element,
            "navigation element capture must finish before its page capture"
        );
        NavigationCapturedScene {
            layers,
            sources: capture.sources,
            destinations: capture.destinations,
            leading_reserve: 0.0,
        }
    }

    pub(crate) fn begin_navigation_element_capture(&mut self) -> bool {
        let Some(capture) = self.navigation_captures.last_mut() else {
            return false;
        };
        assert!(
            !capture.capturing_element,
            "navigation transition metadata cannot be nested"
        );
        capture.capturing_element = true;
        true
    }

    pub(crate) fn finish_navigation_element_capture(
        &mut self,
        source: bool,
        id: Id,
        bounds: kurbo::Rect,
        layers: CapturedLayers,
    ) {
        let capture = self
            .navigation_captures
            .last_mut()
            .expect("navigation element capture requires an active page capture");
        assert!(
            capture.capturing_element,
            "navigation element capture was not started"
        );
        capture.capturing_element = false;
        let elements = if source {
            &mut capture.sources
        } else {
            &mut capture.destinations
        };
        let previous = elements.insert(id, NavigationMatchedElement { bounds, layers });
        assert!(
            previous.is_none(),
            "navigation transition id {id:?} was declared more than once in one page"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        NavigationCapturedScene, NavigationInteractivePop, NavigationInteractivePopOutcome,
        NavigationInteractivePopPhase,
    };
    use std::time::{Duration, Instant};
    use waterui_backend_core::widget::NavigationMotion;
    use waterui_core::EasingCurve;

    fn material_navigation_motion() -> NavigationMotion {
        NavigationMotion {
            transition_duration: Duration::from_millis(450),
            transition_easing: EasingCurve::bezier(0.2, 0.0, 0.0, 1.0),
            shared_axis_slide_distance: 30.0,
            fade_through_threshold: 0.35,
        }
    }

    #[test]
    fn interactive_completion_uses_emphasized_easing_after_direct_manipulation() {
        let started_at = Instant::now();
        let mut pop = NavigationInteractivePop::new(
            0.0,
            100.0,
            NavigationCapturedScene::default(),
            NavigationCapturedScene::default(),
        );
        assert!(pop.update(60.0));
        pop.finish(started_at, NavigationInteractivePopOutcome::ByProgress);

        let (progress, completed, cancelled) = pop.sample(
            started_at + Duration::from_millis(90),
            material_navigation_motion(),
        );

        assert!(
            progress > 0.8,
            "emphasized easing must advance beyond linear progress"
        );
        assert!(!completed);
        assert!(!cancelled);
    }

    #[test]
    fn interactive_cancellation_uses_emphasized_easing_toward_origin() {
        let started_at = Instant::now();
        let mut pop = NavigationInteractivePop::new(
            0.0,
            100.0,
            NavigationCapturedScene::default(),
            NavigationCapturedScene::default(),
        );
        assert!(pop.update(40.0));
        pop.finish(started_at, NavigationInteractivePopOutcome::ByProgress);

        let (progress, completed, cancelled) = pop.sample(
            started_at + Duration::from_millis(90),
            material_navigation_motion(),
        );

        assert!(
            progress < 0.2,
            "emphasized easing must retreat beyond linear progress"
        );
        assert!(!completed);
        assert!(!cancelled);
    }

    #[test]
    fn explicit_commit_completes_below_the_drag_threshold() {
        let started_at = Instant::now();
        let mut pop = NavigationInteractivePop::new(
            0.0,
            100.0,
            NavigationCapturedScene::default(),
            NavigationCapturedScene::default(),
        );
        assert!(pop.set_progress(0.2));
        pop.finish(started_at, NavigationInteractivePopOutcome::Commit);
        assert!(matches!(
            pop.phase,
            NavigationInteractivePopPhase::Completing { .. }
        ));
    }

    #[test]
    fn explicit_cancel_retreats_above_the_drag_threshold() {
        let started_at = Instant::now();
        let mut pop = NavigationInteractivePop::new(
            0.0,
            100.0,
            NavigationCapturedScene::default(),
            NavigationCapturedScene::default(),
        );
        assert!(pop.set_progress(0.8));
        pop.finish(started_at, NavigationInteractivePopOutcome::Cancel);
        assert!(matches!(
            pop.phase,
            NavigationInteractivePopPhase::Cancelling { .. }
        ));
    }
}
