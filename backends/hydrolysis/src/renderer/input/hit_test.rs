use super::*;
use crate::widgets::util::widget_theme;
use waterui_backend_core::widget::WidgetInteractionState;

#[derive(Clone)]
pub(crate) struct PointerTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) captures_drag: bool,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) press_slot: Option<PressSlot>,
    pub(crate) action: PointerAction,
}

#[derive(Clone)]
pub(crate) struct CursorTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) style: CursorStyle,
}

#[derive(Clone)]
pub(crate) struct HoverTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) slot: HoverSlot,
    pub(crate) on_enter: Option<HoverAction>,
    pub(crate) on_move: Option<HoverMoveAction>,
    pub(crate) on_exit: Option<HoverAction>,
}

#[derive(Clone)]
pub(crate) struct ScrollTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) action: ScrollAction,
}

pub(crate) type PointerAction =
    Rc<RefCell<dyn FnMut(&mut HydrolysisRenderer, vello::kurbo::Point, &Environment) -> bool>>;
pub(crate) type HoverAction = Rc<RefCell<dyn FnMut(&Environment) -> bool>>;
pub(crate) type HoverMoveAction = Rc<RefCell<dyn FnMut(vello::kurbo::Point, &Environment) -> bool>>;
pub(crate) type ScrollAction = Rc<RefCell<dyn FnMut(f32, f32, bool) -> bool>>;

#[derive(Default)]
pub(crate) struct HitTestState {
    pub(crate) pointer_targets: Vec<PointerTarget>,
    pub(crate) active_pointer_drag_target: Option<PointerAction>,
    pub(crate) active_pointer_drag_signature: Option<(usize, usize)>,
    pub(crate) cursor_targets: Vec<CursorTarget>,
    pub(crate) hover_targets: Vec<HoverTarget>,
    pub(crate) context_menu_targets: Vec<ContextMenuTarget>,
    pub(crate) interaction: InteractionEngine,
    pub(crate) active_press_bounds: Option<vello::kurbo::Rect>,
    pub(crate) active_press_origin: Option<vello::kurbo::Point>,
    pub(crate) scroll_targets: Vec<ScrollTarget>,
    pub(crate) hit_test_opacity: f32,
    pub(crate) hit_test_order: usize,
}

impl HitTestState {
    pub(crate) fn reset_scene(&mut self) {
        self.pointer_targets.clear();
        self.cursor_targets.clear();
        self.hover_targets.clear();
        self.context_menu_targets.clear();
        self.scroll_targets.clear();
    }

    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.hit_test_opacity = 1.0;
        self.hit_test_order = 0;
        self.interaction.begin_rebuild_frame();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.interaction.finish_rebuild_frame();
    }

    pub(crate) fn next_hit_test_order(&mut self) -> usize {
        let order = self.hit_test_order;
        self.hit_test_order = self
            .hit_test_order
            .checked_add(1)
            .expect("hydrolysis hit-test order overflow");
        order
    }

    pub(crate) fn cursor_style_at(&self, point: vello::kurbo::Point) -> CursorStyle {
        self.cursor_targets
            .iter()
            .rev()
            .find(|target| target.bounds.contains(point))
            .map_or(CursorStyle::Arrow, |target| target.style)
    }

    pub(crate) fn sync_hover_targets(
        &mut self,
        point: vello::kurbo::Point,
        env: &Environment,
        dispatch_move: bool,
    ) -> bool {
        let mut changed = false;
        for target in &mut self.hover_targets {
            let contains = target.bounds.contains(point);
            let slot_hovering = self.interaction.hovering(target.slot);
            if contains && !slot_hovering {
                self.interaction.set_hovering(target.slot, true);
                if let Some(on_enter) = target.on_enter.as_mut() {
                    changed |= (on_enter.borrow_mut())(env);
                }
            } else if !contains && slot_hovering {
                self.interaction.set_hovering(target.slot, false);
                if let Some(on_exit) = target.on_exit.as_mut() {
                    changed |= (on_exit.borrow_mut())(env);
                }
            }
            if contains
                && dispatch_move
                && let Some(on_move) = target.on_move.as_mut()
            {
                changed |= (on_move.borrow_mut())(point, env);
            }
        }
        changed
    }
}

impl HydrolysisRenderer {
    pub(crate) fn sync_active_pointer_drag_target_after_layout(
        &mut self,
        pointer: Option<vello::kurbo::Point>,
    ) {
        let Some(active) = self.hit_test.active_pointer_drag_target.as_ref() else {
            return;
        };
        let alive = self
            .hit_test
            .pointer_targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.action, active));
        if alive {
            return;
        }
        if let Some((depth, order)) = self.hit_test.active_pointer_drag_signature
            && let Some(target) = self.hit_test.pointer_targets.iter().find(|target| {
                target.captures_drag && target.depth == depth && target.order == order
            })
        {
            self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
            return;
        }
        let Some(point) = pointer else {
            self.hit_test.active_pointer_drag_target = None;
            self.hit_test.active_pointer_drag_signature = None;
            return;
        };
        let mut indices: Vec<usize> = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.captures_drag && target.bounds.contains(point))
            .map(|(index, _)| index)
            .collect();
        indices.sort_unstable_by(|left, right| {
            let left_target = &self.hit_test.pointer_targets[*left];
            let right_target = &self.hit_test.pointer_targets[*right];
            Self::target_hit_priority(right_target.depth, right_target.order, *right).cmp(
                &Self::target_hit_priority(left_target.depth, left_target.order, *left),
            )
        });
        if let Some(index) = indices.first().copied() {
            let target = &self.hit_test.pointer_targets[index];
            self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
            self.hit_test.active_pointer_drag_signature = Some((target.depth, target.order));
        } else {
            self.hit_test.active_pointer_drag_target = None;
            self.hit_test.active_pointer_drag_signature = None;
        }
    }

    pub fn handle_pointer_down(
        &mut self,
        x: f32,
        y: f32,
        button: PointerButton,
        env: &Environment,
    ) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant();
        let mut rebuild_requested = false;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_signature = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        rebuild_requested |= self.hit_test.interaction.clear_all_presses(at);
        self.text_editing.active_text_selection_drag = None;
        let overlay_hit = matches!(
            self.text_editing.active_text_context_menu,
            Some(ActiveTextContextMenu::Overlay {
                index: _,
                overlay: _
            })
        );
        if overlay_hit {
            let changed = self.handle_text_context_menu_overlay_pointer_down(point);
            if changed || self.text_editing.active_text_context_menu.is_none() {
                return changed;
            }
        }
        if button != PointerButton::Secondary {
            self.dismiss_active_text_context_menu();
        }
        self.dismiss_active_popup_menu();
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            button = ?button,
            pointer_targets = self.hit_test.pointer_targets.len(),
            text_inputs = self.text_editing.text_input_targets.len(),
            gesture_targets = self.gesture_engine.target_count(),
            "pointer down begin"
        );

        let gesture_changed = self.gesture_engine.handle_pointer_down(point, at, env);
        rebuild_requested |= gesture_changed;

        let mut pointer_indices: Vec<usize> = self
            .hit_test
            .pointer_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .map(|(index, _)| index)
            .collect();
        pointer_indices.sort_unstable_by(|left, right| {
            let left_target = &self.hit_test.pointer_targets[*left];
            let right_target = &self.hit_test.pointer_targets[*right];
            Self::target_hit_priority(right_target.depth, right_target.order, *right).cmp(
                &Self::target_hit_priority(left_target.depth, left_target.order, *left),
            )
        });
        let focused = self.topmost_text_input_index_at_point(point);
        let top_pointer_priority = pointer_indices.first().map(|index| {
            let target = &self.hit_test.pointer_targets[*index];
            Self::target_hit_priority(target.depth, target.order, *index)
        });
        let focused_priority = focused.map(|index| {
            let target = &self.text_editing.text_input_targets[index];
            Self::target_hit_priority(target.depth, target.order, index)
        });
        let focus_wins = matches!(
            (focused_priority, top_pointer_priority),
            (Some(focus_priority), Some(pointer_priority)) if focus_priority > pointer_priority
        ) || matches!((focused_priority, top_pointer_priority), (Some(_), None));
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            button = ?button,
            pointer_hits = ?pointer_indices,
            pointer_top_priority = ?top_pointer_priority,
            focused_candidate = ?focused,
            focused_priority = ?focused_priority,
            focus_wins,
            "pointer down candidates"
        );
        if focus_wins {
            let mut changed = self.set_focused_text_input(focused);
            if let Some(index) = focused {
                match button {
                    PointerButton::Primary => {
                        let click_count = self.next_text_selection_click_count(index, point, at);
                        changed |=
                            self.apply_text_selection_click_gesture(index, point, click_count);
                        self.text_editing.active_text_selection_drag = Some(index);
                    }
                    PointerButton::Secondary => {
                        let keep_selection = {
                            let target = &self.text_editing.text_input_targets[index];
                            let selection_index =
                                Self::text_selection_index_from_point(target, point);
                            let slot = target.selection.borrow();
                            selection_range_contains_index(&target.model, &slot, selection_index)
                        };
                        if !keep_selection {
                            changed |= self.update_text_selection_from_pointer(index, point, false);
                        }
                        changed |= self.show_text_context_menu(index, point, env);
                    }
                    _ => {}
                }
            }
            return rebuild_requested || changed;
        }

        if button != PointerButton::Primary {
            if button == PointerButton::Secondary
                && let Some(target) = self.topmost_context_menu_target_at_point(point)
            {
                if self.set_focused_text_input(focused) {
                    rebuild_requested = true;
                }
                let changed = self.show_popup_menu_nodes(
                    popup_menu_nodes(&target.items.get()),
                    LayoutPoint::new(point.x as f32, point.y as f32),
                    env,
                );
                return rebuild_requested || changed;
            }
            if self.set_focused_text_input(focused) {
                rebuild_requested = true;
            }
            return rebuild_requested;
        }

        for index in pointer_indices {
            let target = self.hit_test.pointer_targets[index].clone();
            if let Some(slot) = target.press_slot {
                self.hit_test.interaction.begin_press(slot, point, at);
                self.hit_test.active_press_bounds = Some(target.bounds);
                self.hit_test.active_press_origin = Some(point);
                rebuild_requested = true;
            }
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                x,
                y,
                pointer_index = index,
                captures_drag = target.captures_drag,
                bounds = ?target.bounds,
                order = target.order,
                "dispatch pointer target"
            );
            let changed = (target.action.borrow_mut())(self, point, env);
            if changed {
                self.request_rebuild();
                rebuild_requested = true;
            }
            if target.captures_drag {
                self.hit_test.active_pointer_drag_target = Some(Rc::clone(&target.action));
                self.hit_test.active_pointer_drag_signature = Some((target.depth, target.order));
            }
            if !changed && !target.captures_drag {
                continue;
            }
            tracing::trace!(
                target: "waterui::hydrolysis::input",
                x,
                y,
                pointer_index = index,
                captures_drag = target.captures_drag,
                order = target.order,
                "pointer target handled event"
            );
            return rebuild_requested || changed;
        }
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            focused_candidate = ?focused,
            "pointer down text-input fallback"
        );
        if self.set_focused_text_input(focused) {
            rebuild_requested = true;
        }
        rebuild_requested
    }

    pub fn handle_pointer_up(
        &mut self,
        x: f32,
        y: f32,
        _button: PointerButton,
        env: &Environment,
    ) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant();
        let mut changed = self.handle_pointer_move(x, y, env);
        self.text_editing.active_text_selection_drag = None;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_signature = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        changed |= self.hit_test.interaction.clear_all_presses(at);
        let gesture_changed = self.gesture_engine.handle_pointer_up(point, at, env);
        changed |= gesture_changed;
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed,
            gesture_changed,
            "pointer up handled"
        );
        changed
    }

    pub fn handle_pointer_move(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant();
        let mut rebuild_requested = false;
        let mut drag_changed = false;
        if let Some(index) = self.text_editing.active_text_selection_drag {
            let text_drag_changed = self.update_text_selection_from_pointer(index, point, true);
            drag_changed |= text_drag_changed;
            rebuild_requested |= text_drag_changed;
        }
        if let Some(action) = self.hit_test.active_pointer_drag_target.clone() {
            let pointer_drag_changed = (action.borrow_mut())(self, point, env);
            if pointer_drag_changed {
                self.request_rebuild();
            }
            drag_changed |= pointer_drag_changed;
            rebuild_requested |= pointer_drag_changed;
        }
        let gesture_changed = self.gesture_engine.handle_pointer_move(point, at, env);
        rebuild_requested |= gesture_changed;
        rebuild_requested |= self.hit_test.sync_hover_targets(point, env, true);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed = rebuild_requested,
            drag_changed,
            gesture_changed,
            dragging = self.hit_test.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer move handled"
        );
        rebuild_requested
    }

    pub fn sync_pointer_hover_state(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let changed = self.hit_test.sync_hover_targets(point, env, false);
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed,
            dragging = self.hit_test.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer hover sync handled"
        );
        changed
    }

    pub fn handle_pointer_cancel(&mut self, env: &Environment) -> bool {
        let at = self.frame_instant();
        let mut rebuild_requested = false;
        self.text_editing.active_text_selection_drag = None;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_signature = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        rebuild_requested |= self.hit_test.interaction.clear_all_presses(at);
        let gesture_changed = self
            .gesture_engine
            .handle_pointer_cancel(self.frame_instant(), env);
        rebuild_requested |= gesture_changed;
        for target in &mut self.hit_test.hover_targets {
            let hovering = self.hit_test.interaction.hovering(target.slot);
            if !hovering {
                continue;
            }
            self.hit_test.interaction.set_hovering(target.slot, false);
            if let Some(on_exit) = target.on_exit.as_mut() {
                rebuild_requested |= (on_exit.borrow_mut())(env);
            }
        }
        rebuild_requested
    }

    pub fn handle_scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        for target in self.hit_test.scroll_targets.iter_mut().rev() {
            if target.bounds.contains(point) {
                return (target.action.borrow_mut())(dx, dy, is_line_delta);
            }
        }
        false
    }

    pub(crate) fn register_pointer_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&mut HydrolysisRenderer, vello::kurbo::Point, &Environment) -> bool,
    {
        self.register_pointer_target_action(
            bounds,
            false,
            None,
            Rc::new(RefCell::new(action)),
            self.render_depth,
        );
    }

    pub(crate) fn register_pointer_target_action(
        &mut self,
        bounds: vello::kurbo::Rect,
        captures_drag: bool,
        press_slot: Option<PressSlot>,
        action: PointerAction,
        depth: usize,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag,
            depth,
            order,
            press_slot,
            action,
        });
    }

    pub(crate) fn bind_interaction_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
    ) -> (WidgetInteractionState, PressSlot) {
        self.bind_interaction_target_with_focus(bounds, env, None)
    }

    pub(crate) fn bind_focused_interaction_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        focused: bool,
    ) -> (WidgetInteractionState, PressSlot) {
        self.bind_interaction_target_with_focus(
            bounds,
            env,
            Some(InteractionFocus::visible(focused)),
        )
    }

    fn bind_interaction_target_with_focus(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        focus: Option<InteractionFocus>,
    ) -> (WidgetInteractionState, PressSlot) {
        let (hover_slot, hovered) = self.hit_test.interaction.bind_hover();
        if self.hit_test.hit_test_opacity > HIT_TEST_ALPHA_THRESHOLD {
            self.hit_test.hover_targets.push(HoverTarget {
                bounds,
                slot: hover_slot,
                on_enter: None,
                on_move: None,
                on_exit: None,
            });
        }
        let motion = widget_theme(env).interaction_motion();
        let now = self.frame_instant();
        self.hit_test.interaction.bind_widget_state(
            bounds,
            hovered,
            focus,
            self.hit_test.active_press_origin,
            &motion,
            &mut self.animation_controller,
            now,
        )
    }

    pub(crate) fn register_interactive_pointer_target<F>(
        &mut self,
        bounds: vello::kurbo::Rect,
        press_slot: PressSlot,
        action: F,
    ) where
        F: 'static + FnMut(&mut HydrolysisRenderer, vello::kurbo::Point, &Environment) -> bool,
    {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag: false,
            depth: self.render_depth,
            order,
            press_slot: Some(press_slot),
            action: Rc::new(RefCell::new(action)),
        });
    }

    pub(crate) fn register_interactive_pointer_drag_target<F>(
        &mut self,
        bounds: vello::kurbo::Rect,
        press_slot: PressSlot,
        action: F,
    ) where
        F: 'static + FnMut(&mut HydrolysisRenderer, vello::kurbo::Point, &Environment) -> bool,
    {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag: true,
            depth: self.render_depth,
            order,
            press_slot: Some(press_slot),
            action: Rc::new(RefCell::new(action)),
        });
    }

    pub(crate) fn ensure_active_pointer_drag_target_is_live(&mut self) {
        let Some(active) = self.hit_test.active_pointer_drag_target.as_ref() else {
            return;
        };
        let alive = self
            .hit_test
            .pointer_targets
            .iter()
            .any(|target| Rc::ptr_eq(&target.action, active));
        if !alive {
            self.hit_test.active_pointer_drag_target = None;
            self.hit_test.active_pointer_drag_signature = None;
        }
    }

    pub(crate) fn register_cursor_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        style: CursorStyle,
    ) {
        self.register_cursor_target_style(bounds, style);
    }

    pub(crate) fn register_cursor_target_style(
        &mut self,
        bounds: vello::kurbo::Rect,
        style: CursorStyle,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        self.hit_test
            .cursor_targets
            .push(CursorTarget { bounds, style });
    }

    pub(crate) fn register_hover_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        on_enter: Option<HoverAction>,
        on_move: Option<HoverMoveAction>,
        on_exit: Option<HoverAction>,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let (slot, _hovering) = self.hit_test.interaction.bind_hover();
        self.hit_test.hover_targets.push(HoverTarget {
            bounds,
            slot,
            on_enter,
            on_move,
            on_exit,
        });
    }

    pub(crate) fn register_hover_enter_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.register_hover_target(bounds, Some(Rc::new(RefCell::new(action))), None, None);
    }

    pub(crate) fn register_hover_exit_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(&Environment) -> bool,
    {
        self.register_hover_target(bounds, None, None, Some(Rc::new(RefCell::new(action))));
    }

    pub(crate) fn register_hover_move_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(vello::kurbo::Point, &Environment) -> bool,
    {
        self.register_hover_target(bounds, None, Some(Rc::new(RefCell::new(action))), None);
    }

    pub(crate) fn register_scroll_target<F>(&mut self, bounds: vello::kurbo::Rect, action: F)
    where
        F: 'static + FnMut(f32, f32, bool) -> bool,
    {
        self.register_scroll_target_action(bounds, Rc::new(RefCell::new(action)));
    }

    pub(crate) fn register_scroll_target_action(
        &mut self,
        bounds: vello::kurbo::Rect,
        action: ScrollAction,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        self.hit_test
            .scroll_targets
            .push(ScrollTarget { bounds, action });
    }
}
