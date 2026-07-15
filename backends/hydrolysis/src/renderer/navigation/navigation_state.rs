use super::*;
use waterui::navigation::{
    AnyNavigationTransition, NavigationDestinationState, NavigationTransactionId,
    NavigationTransitionDirection,
};
use waterui_core::id::Id;

const ROOT_NAVIGATION_IDENTITY: u64 = 0;

#[derive(Clone)]
pub(crate) struct NavigationMatchedElement {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) scene: vello::Scene,
}

#[derive(Clone, Default)]
pub(crate) struct NavigationCapturedScene {
    pub(crate) scene: vello::Scene,
    pub(crate) sources: BTreeMap<Id, NavigationMatchedElement>,
    pub(crate) destinations: BTreeMap<Id, NavigationMatchedElement>,
}

impl NavigationCapturedScene {
    pub(crate) fn composed(&self) -> vello::Scene {
        let mut scene = self.scene.clone();
        for element in self.sources.values().chain(self.destinations.values()) {
            scene.append(&element.scene, None);
        }
        scene
    }

    pub(crate) fn composed_without(&self, source: bool, id: Id) -> vello::Scene {
        let mut scene = self.scene.clone();
        for (element_id, element) in &self.sources {
            if !source || *element_id != id {
                scene.append(&element.scene, None);
            }
        }
        for (element_id, element) in &self.destinations {
            if source || *element_id != id {
                scene.append(&element.scene, None);
            }
        }
        scene
    }
}

#[derive(Default)]
pub(crate) struct NavigationSceneCapture {
    sources: BTreeMap<Id, NavigationMatchedElement>,
    destinations: BTreeMap<Id, NavigationMatchedElement>,
    capturing_element: bool,
}

pub(crate) struct HydroNavigationEntry {
    pub(crate) identity: u64,
    pub(crate) content: RetainedSubview,
    pub(crate) state: NavigationDestinationState,
}

impl HydroNavigationEntry {
    fn new(identity: u64, view: NavigationView) -> Self {
        let NavigationView {
            bar,
            content,
            state,
        } = view;
        Self {
            identity,
            content: RetainedSubview::new(AnyView::new(NavigationView {
                bar,
                content,
                state: NavigationDestinationState::default(),
            })),
            state,
        }
    }
}

pub(crate) struct HydroNavigationEvent {
    pub(crate) transaction_id: NavigationTransactionId,
    pub(crate) previous_identity: u64,
    pub(crate) current_identity: u64,
    pub(crate) removed: Vec<HydroNavigationEntry>,
}

pub(crate) type NavigationEntries = Rc<RefCell<Vec<HydroNavigationEntry>>>;
pub(crate) type NavigationEvents = Rc<RefCell<Vec<HydroNavigationEvent>>>;

#[derive(Default)]
pub(crate) struct NavigationState {
    pub(crate) slots: Vec<NavigationSlot>,
    pub(crate) pending_entries: Vec<(usize, NavigationEntries)>,
    pub(crate) cursor: usize,
}

pub(crate) struct NavigationSlot {
    pub(crate) entries: NavigationEntries,
    pub(crate) events: NavigationEvents,
    pub(crate) controller: NavigationController,
    pub(crate) last_depth: usize,
    pub(crate) active_identity: u64,
    pub(crate) root_state: Option<NavigationDestinationState>,
    pub(crate) root_is_active: bool,
    pub(crate) last_scene: Option<NavigationCapturedScene>,
    pub(crate) scene_cache: BTreeMap<u64, NavigationCapturedScene>,
    pub(crate) transition: Option<NavigationTransitionState>,
    pub(crate) pending_removed: Vec<HydroNavigationEntry>,
    pub(crate) pending_appearance: bool,
    pub(crate) pending_transaction_id: Option<NavigationTransactionId>,
    pub(crate) skip_next_pop_transition: bool,
    pub(crate) interactive_pop: Option<NavigationInteractivePop>,
}

#[derive(Clone)]
pub(crate) struct NavigationTransitionState {
    pub(crate) style: AnyNavigationTransition,
    pub(crate) direction: NavigationTransitionDirection,
    pub(crate) from_scene: NavigationCapturedScene,
    pub(crate) to_scene: NavigationCapturedScene,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

pub(crate) struct HydroNavigationController {
    pub(crate) entries: NavigationEntries,
    pub(crate) events: NavigationEvents,
    pub(crate) next_entry_identity: u64,
    pub(crate) signals: FrameSignals,
}

#[derive(Clone, Copy)]
pub(crate) enum NavigationInteractivePopPhase {
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

pub(crate) struct NavigationInteractivePop {
    pub(crate) start_x: f64,
    pub(crate) width: f64,
    pub(crate) progress: f64,
    pub(crate) phase: NavigationInteractivePopPhase,
    pub(crate) from_scene: NavigationCapturedScene,
    pub(crate) to_scene: NavigationCapturedScene,
}

impl NavigationState {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.pending_entries.clear();
        self.cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
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
            pending_removed: Vec::new(),
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

    pub(crate) fn finish(&mut self, now: Instant, cancelled: bool) {
        self.phase = if cancelled || self.progress < 0.5 {
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

    pub(crate) fn sample(&mut self, now: Instant, duration: Duration) -> (f64, bool, bool) {
        match self.phase {
            NavigationInteractivePopPhase::Dragging => (self.progress, false, false),
            NavigationInteractivePopPhase::Completing {
                started_at,
                initial_progress,
            } => {
                let remaining = 1.0 - initial_progress;
                let elapsed = now.saturating_duration_since(started_at).as_secs_f64();
                let span = (duration.as_secs_f64() * remaining).max(f64::EPSILON);
                self.progress = initial_progress + remaining * (elapsed / span).clamp(0.0, 1.0);
                (self.progress, self.progress >= 1.0, false)
            }
            NavigationInteractivePopPhase::Cancelling {
                started_at,
                initial_progress,
            } => {
                let elapsed = now.saturating_duration_since(started_at).as_secs_f64();
                let span = (duration.as_secs_f64() * initial_progress).max(f64::EPSILON);
                self.progress = initial_progress * (1.0 - elapsed / span).clamp(0.0, 1.0);
                (self.progress, false, self.progress <= 0.0)
            }
        }
    }

    pub(crate) const fn is_animating(&self) -> bool {
        !matches!(self.phase, NavigationInteractivePopPhase::Dragging)
    }
}

impl NavigationTransitionState {
    pub(crate) fn new(
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

    pub(crate) fn is_active(&self, now: Instant) -> bool {
        self.progress(now) < 1.0
    }
}

impl CustomNavigationController for HydroNavigationController {
    fn apply(&mut self, transaction: NavigationTransaction) {
        let (transaction_id, retained_prefix, removed, inserted) = transaction.into_parts();
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
        self.signals.request_rebuild();
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

impl HydrolysisRenderer {
    pub(crate) fn install_navigation_root_state(
        &mut self,
        slot_index: usize,
        state: NavigationDestinationState,
    ) {
        let slot = self
            .navigation
            .slots
            .get_mut(slot_index)
            .expect("Hydrolysis navigation slot missing during root installation");
        assert!(
            slot.root_state.is_none(),
            "Hydrolysis navigation root state was installed more than once"
        );
        slot.root_state = Some(state);
    }

    pub(crate) fn activate_navigation_root_if_needed(
        &mut self,
        slot_index: usize,
        env: &Environment,
    ) {
        let should_activate = {
            let slot = self
                .navigation
                .slots
                .get(slot_index)
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
            .get_mut(slot_index)
            .expect("Hydrolysis navigation slot missing during root activation");
        let mut state = slot
            .root_state
            .take()
            .expect("Hydrolysis navigation root state is not installed");
        state.appeared(env);
        slot.root_state = Some(state);
        slot.root_is_active = true;
    }

    pub(crate) fn begin_navigation_scene_capture(&mut self) {
        self.navigation_captures
            .push(NavigationSceneCapture::default());
    }

    pub(crate) fn finish_navigation_scene_capture(
        &mut self,
        scene: vello::Scene,
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
            scene,
            sources: capture.sources,
            destinations: capture.destinations,
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
        bounds: vello::kurbo::Rect,
        scene: vello::Scene,
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
        let previous = elements.insert(id, NavigationMatchedElement { bounds, scene });
        assert!(
            previous.is_none(),
            "navigation transition id {id:?} was declared more than once in one page"
        );
    }

    pub(crate) fn bind_navigation_entries(&mut self) -> (usize, NavigationEntries) {
        let index = self.navigation.cursor;
        self.navigation.cursor = self
            .navigation
            .cursor
            .checked_add(1)
            .expect("navigation slot cursor overflow");

        if index == self.navigation.slots.len() {
            self.navigation
                .slots
                .push(NavigationSlot::new(self.signals.clone()));
        }

        (index, Rc::clone(&self.navigation.slots[index].entries))
    }

    pub(crate) fn push_pending_navigation_entries(
        &mut self,
        slot_index: usize,
        entries: NavigationEntries,
    ) {
        self.navigation.pending_entries.push((slot_index, entries));
    }

    pub(crate) fn take_pending_navigation_entries(
        &mut self,
        caller: &'static str,
    ) -> (usize, NavigationEntries) {
        self.navigation
            .pending_entries
            .pop()
            .unwrap_or_else(|| panic!("hydrolysis {caller} requires prebound navigation entries"))
    }

    pub(crate) fn finish_interactive_navigation_pop(&mut self, cancelled: bool) -> bool {
        let now = self.frame_instant();
        let mut changed = false;
        for slot in &mut self.navigation.slots {
            if let Some(interactive) = slot.interactive_pop.as_mut()
                && matches!(interactive.phase, NavigationInteractivePopPhase::Dragging)
            {
                interactive.finish(now, cancelled);
                changed = true;
            }
        }
        if changed {
            self.signals.request_rebuild();
        }
        changed
    }

    pub(crate) fn attempt_navigation_pop(&mut self, slot_index: usize, env: &Environment) -> bool {
        let slot = self
            .navigation
            .slots
            .get_mut(slot_index)
            .expect("Hydrolysis navigation slot missing during pop attempt");
        let identity = slot.active_identity;
        let Some(mut state) = take_destination_state(slot, identity) else {
            return false;
        };
        let allowed = state.attempt_pop(env);
        put_destination_state(slot, identity, state);
        allowed
    }

    pub(crate) fn navigation_destination_disappeared(
        &mut self,
        slot_index: usize,
        identity: u64,
        env: &Environment,
    ) {
        let slot = self
            .navigation
            .slots
            .get_mut(slot_index)
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

    pub(crate) fn complete_navigation_transaction(&mut self, slot_index: usize, env: &Environment) {
        let (mut removed, appearance_identity, transaction_id, controller) = {
            let slot = self
                .navigation
                .slots
                .get_mut(slot_index)
                .expect("Hydrolysis navigation slot missing during transaction completion");
            let appearance_identity = slot.pending_appearance.then_some(slot.active_identity);
            slot.pending_appearance = false;
            for entry in &slot.pending_removed {
                slot.scene_cache.remove(&entry.identity);
            }
            (
                core::mem::take(&mut slot.pending_removed),
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
                .get_mut(slot_index)
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
