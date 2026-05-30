use super::*;

pub(crate) type NavigationEntries = Rc<RefCell<Vec<AnyViewBuilder<NavigationView>>>>;

#[derive(Default)]
pub(crate) struct NavigationState {
    pub(crate) slots: Vec<NavigationSlot>,
    pub(crate) pending_entries: Vec<(usize, NavigationEntries)>,
    pub(crate) cursor: usize,
}

pub(crate) struct NavigationSlot {
    pub(crate) entries: NavigationEntries,
    pub(crate) controller: NavigationController,
    pub(crate) last_depth: usize,
    pub(crate) last_scene: Option<vello::Scene>,
    pub(crate) transition: Option<NavigationTransitionState>,
}

#[derive(Clone)]
pub(crate) struct NavigationTransitionState {
    pub(crate) style: NavigationTransition,
    pub(crate) direction: NavigationTransitionDirection,
    pub(crate) from_scene: vello::Scene,
    pub(crate) to_scene: vello::Scene,
    pub(crate) started_at: Instant,
    pub(crate) duration: Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavigationTransitionDirection {
    Push,
    Pop,
}

pub(crate) struct HydroNavigationController {
    pub(crate) entries: NavigationEntries,
    pub(crate) rebuild_requested: Rc<Cell<bool>>,
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
    pub(crate) fn new(rebuild_requested: Rc<Cell<bool>>) -> Self {
        let entries = Rc::new(RefCell::new(Vec::new()));
        let controller = NavigationController::new(HydroNavigationController {
            entries: Rc::clone(&entries),
            rebuild_requested,
        });

        Self {
            entries,
            controller,
            last_depth: 0,
            last_scene: None,
            transition: None,
        }
    }
}

impl NavigationTransitionState {
    pub(crate) fn new(
        style: NavigationTransition,
        direction: NavigationTransitionDirection,
        from_scene: vello::Scene,
        to_scene: vello::Scene,
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
    fn push_builder(&mut self, content: AnyViewBuilder<NavigationView>) {
        self.entries.borrow_mut().push(content);
        self.rebuild_requested.set(true);
    }

    fn push(&mut self, _content: NavigationView) {
        panic!(
            "hydrolysis NavigationController::push(NavigationView) is unsupported; use NavigationLink or push_builder"
        );
    }

    fn pop(&mut self) {
        if self.entries.borrow_mut().pop().is_some() {
            self.rebuild_requested.set(true);
        }
    }
}

impl HydrolysisRenderer {
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
                .push(NavigationSlot::new(Rc::clone(&self.rebuild_requested)));
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
}
