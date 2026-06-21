//! Interaction bindings: hit-test/gesture/text-input registration, scroll
//! handle binding, and pointer/IME/focus queries used by the runner.

use super::*;

impl HydrolysisRenderer {
    pub(super) fn target_hit_priority(
        depth: usize,
        order: usize,
        index: usize,
    ) -> (usize, usize, usize) {
        (order, depth, index)
    }

    pub(super) fn topmost_text_input_index_at_point(
        &self,
        point: vello::kurbo::Point,
    ) -> Option<usize> {
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
    pub(super) fn focused_text_input_accessibility_node(&self) -> Option<AccessibilityNodeId> {
        let index = self.text_editing.focused_text_input.get()?;
        let target = self.text_editing.text_input_targets.as_slice().get(index)?;
        target.accessibility_node_id
    }

    #[cfg(feature = "accessibility")]
    pub(super) fn focus_text_input_for_accessibility_node(
        &mut self,
        node_id: AccessibilityNodeId,
    ) -> bool {
        let focused = self
            .text_editing
            .text_input_targets
            .iter()
            .position(|target| target.accessibility_node_id == Some(node_id))
            .unwrap_or_else(|| {
                panic!(
                    "hydrolysis accessibility focus target node {:?} has no matching text input target",
                    node_id
                )
            });
        self.set_focused_text_input(Some(focused))
    }

    pub(crate) fn push_pending_scroll_handle(&mut self, handle: ScrollHandle) {
        self.lazy.push_pending_scroll_handle(handle);
    }

    pub(crate) fn bind_scroll_handle(
        &mut self,
        axis: ScrollAxis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> ScrollHandle {
        let handle = self.scroll_controller.bind(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
        );
        self.push_pending_scroll_handle(handle.clone());
        handle
    }

    pub(crate) fn bind_render_scroll_handle(
        &mut self,
        axis: ScrollAxis,
        viewport_width: f64,
        viewport_height: f64,
        content_width: f64,
        content_height: f64,
    ) -> ScrollHandle {
        self.scroll_controller.bind(
            axis,
            viewport_width,
            viewport_height,
            content_width,
            content_height,
        )
    }

    pub(crate) fn take_pending_scroll_handle(&mut self, caller: &'static str) -> ScrollHandle {
        self.lazy.take_pending_scroll_handle(caller)
    }

    pub(crate) fn push_lazy_viewport(&mut self, viewport: vello::kurbo::Rect) {
        self.lazy.lazy_viewport_stack.push(viewport);
    }

    pub(crate) fn pop_lazy_viewport(&mut self, caller: &'static str) {
        self.lazy
            .lazy_viewport_stack
            .pop()
            .unwrap_or_else(|| panic!("lazy viewport stack underflow in {caller}"));
    }

    pub(crate) fn next_text_input_index(&self) -> usize {
        self.text_editing.text_input_targets.len()
    }

    pub(crate) fn is_text_input_focused(&self, index: usize) -> bool {
        self.text_editing.focused_text_input.get() == Some(index)
    }

    pub(crate) fn current_ime_preedit(&self) -> Option<Str> {
        self.text_editing.ime_preedit.clone()
    }

    #[must_use]
    pub fn focused_text_input_state(&self) -> Option<TextInputState> {
        let index = self.text_editing.focused_text_input.get()?;
        let target = self.text_editing.text_input_targets.as_slice().get(index)?;
        Some(TextInputState {
            x: target.cursor_area.x0,
            y: target.cursor_area.y0,
            width: target.cursor_area.width().max(1.0),
            height: target.cursor_area.height().max(1.0),
            purpose: target.purpose,
        })
    }

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
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
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
        let center = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant;
        self.gesture_engine
            .handle_magnification(center, delta, phase, at, env)
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
        let center = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant;
        self.gesture_engine
            .handle_rotation(center, delta, phase, at, env)
    }

    pub fn handle_gesture_tick(&mut self, at: Instant, env: &Environment) -> bool {
        self.gesture_engine.handle_tick(at, env)
    }

    pub fn next_gesture_deadline(&self) -> Option<Instant> {
        let gesture_deadline = self.gesture_engine.next_deadline();
        let caret_deadline = self
            .text_editing
            .focused_text_input
            .get()
            .and(self.text_editing.text_caret_next_frame_at);
        match (gesture_deadline, caret_deadline) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (Some(left), None) => Some(left),
            (None, Some(right)) => Some(right),
            (None, None) => None,
        }
    }

    pub fn sync_active_interactions_after_layout(&mut self, pointer: Option<(f32, f32)>) {
        let pointer = pointer.map(|(x, y)| vello::kurbo::Point::new(f64::from(x), f64::from(y)));
        self.gesture_engine.sync_after_layout(pointer);
        self.sync_active_pointer_drag_target_after_layout(pointer);
    }

    pub(super) fn register_gesture_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        group_id: usize,
        gesture: Gesture,
        action: BoxedAction<()>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.gesture_engine.register_target(
            bounds,
            gesture,
            action,
            self.render_depth,
            order,
            group_id,
        );
    }

    pub(super) fn allocate_gesture_group_id(&mut self) -> usize {
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
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.next_hit_test_order();
        self.text_editing.text_input_targets.push(TextInputTarget {
            bounds: data.target.bounds,
            cursor_area: data.target.cursor_area,
            text_bounds: data.target.text_bounds,
            text_clip_bounds: data.target.text_clip_bounds,
            content_alpha: data.target.content_alpha,
            layout: data.target.layout,
            purpose: data.target.purpose,
            depth: data.depth,
            order,
            model: data.target.model,
            selection: data.target.selection,
            focus_binding: data.focus_binding,
            #[cfg(feature = "accessibility")]
            accessibility_node_id: data.accessibility_node_id,
        });
    }
}
