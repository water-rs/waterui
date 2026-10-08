//! Interaction bindings: hit-test/gesture/text-input registration, scroll
//! handle binding, and pointer/IME/focus queries used by the runner.

// glob import of the module vocabulary — the renderer internals are designed to be used wholesale
#[allow(clippy::wildcard_imports)]
use super::*;

impl SemanticCore {
    pub(super) const fn target_hit_priority(
        depth: usize,
        order: usize,
        index: usize,
    ) -> (usize, usize, usize) {
        (order, depth, index)
    }

    pub(super) fn topmost_text_input_index_at_point(&self, point: kurbo::Point) -> Option<usize> {
        self.text_editing
            .text_input_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                Self::target_hit_priority(left.depth, left.order, *left_index).cmp(
                    &Self::target_hit_priority(right.depth, right.order, *right_index),
                )
            })
            .map(|(index, _)| index)
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn focused_text_input_accessibility_node(&self) -> Option<AccessibilityNodeId> {
        self.text_editing.focused_target()?.accessibility_node_id
    }

    /// Whether a text input claims this accessibility node — the check the
    /// Android runner runs before the panicking focus call.
    #[cfg(all(feature = "accessibility", target_os = "android"))]
    pub(crate) fn accessibility_node_is_text_input(&self, node_id: AccessibilityNodeId) -> bool {
        self.text_editing
            .text_input_targets
            .iter()
            .any(|target| target.accessibility_node_id == Some(node_id))
    }

    #[cfg(feature = "accessibility")]
    pub(crate) fn focus_text_input_for_accessibility_node(
        &mut self,
        node_id: AccessibilityNodeId,
    ) -> bool {
        self.materialize_registries();
        let focused = self
            .text_editing
            .text_input_targets
            .iter()
            .position(|target| target.accessibility_node_id == Some(node_id))
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis accessibility focus target node {node_id:?} has no matching text input target"
                )
            });
        self.set_focused_text_input(Some(focused))
    }

    pub(crate) fn push_lazy_viewport(&mut self, viewport: LazyViewport) {
        self.lazy.lazy_viewport_stack.push(viewport);
    }

    pub(crate) fn pop_lazy_viewport(&mut self, caller: &'static str) {
        self.lazy
            .lazy_viewport_stack
            .pop()
            .unwrap_or_else(|| panic!("lazy viewport stack underflow in {caller}"));
    }

    /// Whether the text input with this stable identity holds focus. Asked by a
    /// field while it is being flushed, before it has registered its target, so
    /// it must not depend on a position in this frame's target list.
    pub(crate) fn is_text_input_focused(&self, key: &InteractionKey) -> bool {
        self.text_editing.is_focused(key)
    }

    pub(crate) fn current_ime_preedit(&self) -> Option<Str> {
        self.text_editing.ime_preedit.clone()
    }

    /// The platform-reported caret inside [`Self::current_ime_preedit`], so a
    /// field can map it onto the composed text's layout.
    pub(crate) fn current_ime_preedit_caret(&self) -> Option<usize> {
        self.text_editing.ime_preedit.as_ref()?;
        self.text_editing.ime_preedit_caret
    }

    /// Whether an IME composition currently owns keyboard input — either a
    /// widget field holding marked text or an embedded surface's session.
    pub(crate) const fn ime_composition_active(&self) -> bool {
        self.text_editing.ime_preedit.is_some() || self.hit_test.embedded_composing
    }

    /// Records a key press the IME consumed while it owned input; the press's
    /// release must be swallowed when it arrives, however many batches later.
    pub(crate) fn swallow_ime_key_press(&mut self, code: keyboard_types::Code) {
        self.ime_swallowed_codes.push(code);
    }

    /// True once for a release matching a swallowed press: `wl_keyboard` (and
    /// X11's filtered-key quirk) still deliver it, but it belongs to the
    /// composition, not to the application.
    pub(crate) fn take_ime_swallowed_release(&mut self, code: keyboard_types::Code) -> bool {
        let Some(index) = self.ime_swallowed_codes.iter().position(|c| *c == code) else {
            return false;
        };
        self.ime_swallowed_codes.swap_remove(index);
        true
    }

    /// Where the platform should anchor the input-method panel.
    ///
    /// A focused embedded surface draws its own caret, so it answers first:
    /// its view reports the caret in surface-local logical coordinates and the
    /// live target projects that back into the window.
    #[must_use]
    pub fn focused_text_input_state(&self) -> Option<TextInputState> {
        if let Some(caret) = self.focused_embedded_ime_caret() {
            return Some(TextInputState {
                x: caret.x0,
                y: caret.y0,
                width: caret.width().max(1.0),
                height: caret.height().max(1.0),
                purpose: crate::platform::TextInputPurpose::Normal,
                activation: self.text_editing.activations(),
            });
        }
        let target = self.text_editing.focused_target()?;
        Some(TextInputState {
            x: target.cursor_area.x0,
            y: target.cursor_area.y0,
            width: target.cursor_area.width().max(1.0),
            height: target.cursor_area.height().max(1.0),
            purpose: target.purpose,
            activation: self.text_editing.activations(),
        })
    }

    /// The focused text input's accessibility node — UI focus is the text
    /// caret's home, deliberately separate from the semantic tree's focus
    /// (`accessibility.focus`, reported as `TreeUpdate::focus`): the tree's
    /// focus landing on a non-text node leaves the caret on the field it
    /// belongs to, while a move that carries keyboard focus — traversal or
    /// a pointer press — ends editing (#95).
    #[cfg(feature = "accessibility")]
    #[must_use]
    pub fn focused_ui_node(&self) -> Option<AccessibilityNodeId> {
        self.focused_text_input_accessibility_node()
    }

    pub fn clear_ui_focus(&mut self) -> bool {
        self.set_focused_text_input(None)
    }

    #[must_use]
    pub fn cursor_style_at(&self, x: f32, y: f32) -> CursorStyle {
        let point = kurbo::Point::new(f64::from(x), f64::from(y));
        self.hit_test.cursor_style_at(point)
    }

    pub fn handle_magnification(
        &mut self,
        x: f32,
        y: f32,
        delta: f32,
        phase: TouchPhase,
        env: &Environment,
    ) -> bool {
        let center = kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant;
        self.with_unoccluded_gesture_targets(center, |engine| {
            engine.handle_magnification(center, delta, phase, at, env)
        })
    }

    pub fn apply_magnification_gesture(
        &mut self,
        x: f32,
        y: f32,
        factor: f32,
        env: &Environment,
    ) -> bool {
        assert!(
            factor.is_finite() && factor > 0.0,
            "hydrolysis magnification factor must be finite and positive"
        );
        let mut changed = self.handle_magnification(x, y, 0.0, TouchPhase::Started, env);
        changed |= self.handle_magnification(x, y, factor - 1.0, TouchPhase::Moved, env);
        changed |= self.handle_magnification(x, y, 0.0, TouchPhase::Ended, env);
        changed
    }

    pub fn handle_rotation(
        &mut self,
        x: f32,
        y: f32,
        delta: f32,
        phase: TouchPhase,
        env: &Environment,
    ) -> bool {
        let center = kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant;
        self.with_unoccluded_gesture_targets(center, |engine| {
            engine.handle_rotation(center, delta, phase, at, env)
        })
    }

    pub fn handle_gesture_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.gesture_engine.handle_tick(at, env)
    }

    pub fn next_gesture_deadline(&self) -> Option<Instant> {
        let gesture_deadline = self.gesture_engine.next_deadline();
        let caret_deadline = self
            .text_editing
            .has_focus()
            .then_some(self.text_editing.text_caret_next_frame_at)
            .flatten();
        // An armed context-menu hold wakes the runner at the same instant
        // its tick fires it — it shares the gesture deadline channel.
        let hold_deadline = self
            .hit_test
            .pending_context_menu_hold
            .map(|hold| hold.started_at + CONTEXT_MENU_HOLD_DURATION);
        [gesture_deadline, caret_deadline, hold_deadline]
            .into_iter()
            .flatten()
            .min()
    }

    pub fn sync_active_interactions_after_layout(&mut self, pointer: Option<(f32, f32)>) {
        let pointer = pointer.map(|(x, y)| kurbo::Point::new(f64::from(x), f64::from(y)));
        self.gesture_engine.sync_after_layout(pointer);
        self.sync_active_pointer_drag_target_after_layout(pointer);
    }

    pub(crate) fn register_gesture_target(
        &mut self,
        bounds: kurbo::Rect,
        group_id: usize,
        gesture: Gesture,
        action: BoxedAction<()>,
    ) -> crate::gesture::GestureTarget {
        // `bounds` is node-local: the recognizer starts with it, and
        // materialization re-registers every gesture at its resolved
        // window rect and merged paint rank.
        let target = self.gesture_engine.register_target(
            bounds,
            gesture,
            action,
            self.render_depth,
            0,
            group_id,
        );
        self.register_retained(
            mount::RegisteredGesture {
                owner: std::rc::Weak::new(),
                target: target.clone(),
                owners: self.owner_stack.clone(),
            },
            bounds,
            |regs| &mut regs.gesture_regions,
        );
        target
    }

    /// Re-registers a gesture target a widget retained from an earlier frame,
    /// at that row's current node-local bounds. The recognizer state machine is
    /// shared, so a drag that began before this frame keeps running.
    ///
    /// The retained record resolves the target's window rect and merged paint
    /// rank at materialization — its birth order is not reused, since a stale
    /// rank would weigh it against siblings it was never painted with.
    pub(crate) fn register_retained_gesture_target(
        &mut self,
        target: &crate::gesture::GestureTarget,
        bounds: kurbo::Rect,
        group_id: usize,
    ) {
        let mut target = target.with_bounds_depth_and_group(bounds, self.render_depth, group_id);
        target.order = 0;
        self.register_retained(
            mount::RegisteredGesture {
                owner: std::rc::Weak::new(),
                target,
                owners: self.owner_stack.clone(),
            },
            bounds,
            |regs| &mut regs.gesture_regions,
        );
    }

    pub(crate) const fn allocate_gesture_group_id(&mut self) -> usize {
        let group_id = self.next_gesture_group_id;
        self.next_gesture_group_id = self
            .next_gesture_group_id
            .checked_add(1)
            .expect("hydrolysis gesture group id overflow");
        group_id
    }

    pub(super) fn gesture_group_id_for_identity(&mut self, identity: usize) -> usize {
        if let Some(group_id) = self.gesture_group_ids.get(&identity).copied() {
            return group_id;
        }
        let group_id = self.allocate_gesture_group_id();
        self.gesture_group_ids.insert(identity, group_id);
        group_id
    }

    pub(crate) fn register_text_input_target(&mut self, target: TextInputTargetRegistration) {
        #[cfg(feature = "accessibility")]
        let accessibility_node_id = self.take_pending_text_input_accessibility_node();
        self.register_text_input_target_data(text_editing::TextInputTargetData {
            target,
            depth: self.render_depth,
            focus_binding: None,
            #[cfg(feature = "accessibility")]
            accessibility_node_id,
        });
    }

    pub(super) fn register_text_input_target_data(
        &mut self,
        data: text_editing::TextInputTargetData,
    ) {
        // All four rects stay node-local; materialization resolves them and
        // applies the clip intersect + alpha gate.
        let local = data.target.bounds;
        let key_handlers = self.snapshot_key_handlers();
        self.register_retained(
            TextInputTarget {
                owner: std::rc::Weak::new(),
                interaction_key: data.target.interaction_key,
                modal: data.target.modal,
                bounds: data.target.bounds,
                frame: data.target.bounds,
                cursor_area: data.target.cursor_area,
                text_bounds: data.target.text_bounds,
                text_clip_bounds: data.target.text_clip_bounds,
                content_alpha: data.target.content_alpha,
                layout: data.target.layout,
                display_text: data.target.display_text,
                display_layout: data.target.display_layout,
                purpose: data.target.purpose,
                depth: data.depth,
                order: 0,
                model: data.target.model,
                selection: data.target.selection,
                env: data.target.env,
                key_handlers,
                focus_binding: data.focus_binding,
                #[cfg(feature = "accessibility")]
                accessibility_node_id: data.accessibility_node_id,
            },
            local,
            |regs| &mut regs.text_input_targets,
        );
    }
}
