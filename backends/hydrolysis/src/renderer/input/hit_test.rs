use super::*;
use crate::widgets::util::widget_theme;
use nami::{Computed, Signal as _};
use waterui::drag_drop::DragData;
use waterui_backend_core::widget::WidgetInteractionState;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct DropTargetKey {
    depth: usize,
    order: usize,
}

#[derive(Clone)]
pub(crate) struct DropTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) key: DropTargetKey,
    pub(crate) env: Environment,
    pub(crate) on_drop: Rc<RefCell<BoxedAction<()>>>,
    pub(crate) on_enter: Option<Rc<RefCell<BoxedAction<()>>>>,
    pub(crate) on_exit: Option<Rc<RefCell<BoxedAction<()>>>>,
}

/// Shareable, pre-wrapped drop-destination handlers. A [`DropDestination`]'s
/// boxed closures are wrapped in `Rc<RefCell<…>>` once (the form the hit-test
/// stores), so a retained `Wrapper` node can hold them by value and re-register
/// the same handles on every flush without moving the closures out.
#[derive(Clone)]
pub(crate) struct DropDestinationHandles {
    on_drop: Rc<RefCell<BoxedAction<()>>>,
    on_enter: Option<Rc<RefCell<BoxedAction<()>>>>,
    on_exit: Option<Rc<RefCell<BoxedAction<()>>>>,
}

impl DropDestinationHandles {
    pub(crate) fn from_destination(destination: DropDestination) -> Self {
        Self {
            on_drop: Rc::new(RefCell::new(destination.on_drop)),
            on_enter: destination
                .on_enter
                .map(|handler| Rc::new(RefCell::new(handler))),
            on_exit: destination
                .on_exit
                .map(|handler| Rc::new(RefCell::new(handler))),
        }
    }
}

pub(crate) struct ActiveDrag {
    pub(crate) data: DragData,
    pub(crate) hovered_target: Option<DropTargetKey>,
}

#[derive(Clone)]
pub(crate) struct PointerTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) captures_drag: bool,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) press_slot: Option<PressSlot>,
    /// Replayable state-layer handles for the widget owning this target, so
    /// press feedback animates without a structural rebuild.
    pub(crate) interaction: Option<Rc<InteractionLayerHandles>>,
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
    /// Replayable state-layer handles for the widget owning this target, so
    /// hover feedback animates without a structural rebuild.
    pub(crate) handles: Option<Rc<InteractionLayerHandles>>,
    pub(crate) on_enter: Option<HoverAction>,
    pub(crate) on_move: Option<HoverMoveAction>,
    pub(crate) on_exit: Option<HoverAction>,
}

#[derive(Clone)]
pub(crate) struct ScrollTarget {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) action: ScrollAction,
    /// The scroll view's offset handle, ticked per frame while a smoothed
    /// wheel scroll glides toward its target.
    pub(crate) handle: crate::scroll::ScrollHandle,
}

/// Outcome of synchronizing hover targets against a pointer position.
///
/// `visual_changed` means a replayable state layer's target changed (a redraw
/// replays it); `handler_changed` means a user hover handler reported a state
/// change (escalates to rebuild like any other action).
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct HoverSync {
    pub(crate) visual_changed: bool,
    pub(crate) handler_changed: bool,
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
    pub(crate) drop_targets: Vec<DropTarget>,
    pub(crate) active_drag: Option<ActiveDrag>,
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
        self.drop_targets.clear();
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
        now: Instant,
    ) -> HoverSync {
        let mut sync = HoverSync::default();
        for target in &mut self.hover_targets {
            let contains = target.bounds.contains(point);
            let slot_hovering = self.interaction.hovering(target.slot);
            if contains != slot_hovering {
                self.interaction.set_hovering(target.slot, contains);
                if let Some(handles) = &target.handles {
                    // Replayable state layer: the hover alpha animates through
                    // its retained draw, no structural rebuild needed unless
                    // the widget's chrome samples interaction state directly.
                    handles.set_hovering(contains, now);
                    if handles.chrome_state_dependent() {
                        sync.handler_changed = true;
                    } else {
                        sync.visual_changed = true;
                    }
                }
                if contains {
                    if let Some(on_enter) = target.on_enter.as_mut() {
                        sync.handler_changed |= (on_enter.borrow_mut())(env);
                    }
                } else if let Some(on_exit) = target.on_exit.as_mut() {
                    sync.handler_changed |= (on_exit.borrow_mut())(env);
                }
            }
            if contains
                && dispatch_move
                && let Some(on_move) = target.on_move.as_mut()
            {
                sync.handler_changed |= (on_move.borrow_mut())(point, env);
            }
        }
        sync
    }

    fn topmost_drop_target_index_at_point(&self, point: vello::kurbo::Point) -> Option<usize> {
        self.drop_targets
            .iter()
            .enumerate()
            .filter(|(_, target)| target.bounds.contains(point))
            .max_by(|(left_index, left), (right_index, right)| {
                HydrolysisRenderer::target_hit_priority(left.key.depth, left.key.order, *left_index)
                    .cmp(&HydrolysisRenderer::target_hit_priority(
                        right.key.depth,
                        right.key.order,
                        *right_index,
                    ))
            })
            .map(|(index, _)| index)
    }

    fn drop_target_with_key(&self, key: DropTargetKey) -> Option<DropTarget> {
        self.drop_targets
            .iter()
            .find(|target| target.key == key)
            .cloned()
    }
}

impl HydrolysisRenderer {
    fn call_drop_action(
        action: &Rc<RefCell<BoxedAction<()>>>,
        captured_env: &Environment,
        runtime_env: &Environment,
        data: &DragData,
    ) {
        let drag_env = runtime_env.extending(data.clone());
        let action_env = captured_env.layered_on(&drag_env);
        (action.borrow_mut())(&action_env);
    }

    fn sync_active_drag_hover(&mut self, point: vello::kurbo::Point, env: &Environment) -> bool {
        let Some(active_drag) = self.hit_test.active_drag.as_ref() else {
            return false;
        };
        let data = active_drag.data.clone();
        let previous_key = active_drag.hovered_target;
        let current_target = self
            .hit_test
            .topmost_drop_target_index_at_point(point)
            .map(|index| self.hit_test.drop_targets[index].clone());
        let current_key = current_target.as_ref().map(|target| target.key);
        if previous_key == current_key {
            return false;
        }

        let previous_target = previous_key.and_then(|key| self.hit_test.drop_target_with_key(key));
        if let Some(active_drag) = self.hit_test.active_drag.as_mut() {
            active_drag.hovered_target = current_key;
        }

        let mut changed = previous_key.is_some() || current_key.is_some();
        if let Some(target) = previous_target
            && let Some(on_exit) = target.on_exit.as_ref()
        {
            Self::call_drop_action(on_exit, &target.env, env, &data);
            changed = true;
        }
        if let Some(target) = current_target
            && let Some(on_enter) = target.on_enter.as_ref()
        {
            Self::call_drop_action(on_enter, &target.env, env, &data);
            changed = true;
        }
        changed
    }

    fn begin_or_update_drag(
        &mut self,
        data: DragData,
        point: vello::kurbo::Point,
        env: &Environment,
    ) -> bool {
        if let Some(active_drag) = self.hit_test.active_drag.as_mut() {
            active_drag.data = data;
        } else {
            self.hit_test.active_drag = Some(ActiveDrag {
                data,
                hovered_target: None,
            });
        }
        self.sync_active_drag_hover(point, env)
    }

    fn finish_active_drag(&mut self, point: vello::kurbo::Point, env: &Environment) -> bool {
        let Some(active_drag) = self.hit_test.active_drag.take() else {
            return false;
        };
        let drop_target = self
            .hit_test
            .topmost_drop_target_index_at_point(point)
            .map(|index| self.hit_test.drop_targets[index].clone());
        let exit_target = active_drag
            .hovered_target
            .and_then(|key| self.hit_test.drop_target_with_key(key));

        let mut changed = false;
        if let Some(target) = drop_target {
            Self::call_drop_action(&target.on_drop, &target.env, env, &active_drag.data);
            changed = true;
        }
        if let Some(target) = exit_target
            && let Some(on_exit) = target.on_exit.as_ref()
        {
            Self::call_drop_action(on_exit, &target.env, env, &active_drag.data);
            changed = true;
        }
        changed
    }

    fn cancel_active_drag(&mut self, env: &Environment) -> bool {
        let Some(active_drag) = self.hit_test.active_drag.take() else {
            return false;
        };
        let exit_target = active_drag
            .hovered_target
            .and_then(|key| self.hit_test.drop_target_with_key(key));
        if let Some(target) = exit_target
            && let Some(on_exit) = target.on_exit.as_ref()
        {
            Self::call_drop_action(on_exit, &target.env, env, &active_drag.data);
            return true;
        }
        false
    }

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
        let mut visual_changed = false;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_signature = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        rebuild_requested |= self.cancel_active_drag(env);
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        visual_changed |= press_clear.visual_changed;
        rebuild_requested |= press_clear.chrome_changed;
        if rebuild_requested {
            self.request_rebuild();
        } else if visual_changed {
            self.request_redraw();
        }
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
                return visual_changed || changed;
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
            return rebuild_requested || visual_changed || changed;
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
                return rebuild_requested || visual_changed || changed;
            }
            if self.set_focused_text_input(focused) {
                rebuild_requested = true;
            }
            return rebuild_requested || visual_changed;
        }

        for index in pointer_indices {
            let target = self.hit_test.pointer_targets[index].clone();
            if let Some(slot) = target.press_slot {
                self.hit_test.interaction.begin_press(slot, point, at);
                self.hit_test.active_press_bounds = Some(target.bounds);
                self.hit_test.active_press_origin = Some(point);
                if target
                    .interaction
                    .as_ref()
                    .is_some_and(|handles| handles.chrome_state_dependent())
                {
                    self.request_rebuild();
                    rebuild_requested = true;
                } else {
                    self.request_redraw();
                }
                visual_changed = true;
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
            // A target with a press slot is an opaque interactive control: it
            // consumes the primary press even when its action reports no state
            // change (a button action's `false` means "no rebuild needed", not
            // "unhandled"). Only slot-less utility targets stay transparent —
            // otherwise one tap would fire every overlapping control beneath
            // the topmost one.
            if !changed && !target.captures_drag && target.press_slot.is_none() {
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
            return rebuild_requested || visual_changed || changed;
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
        rebuild_requested || visual_changed
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
        let drop_changed = self.finish_active_drag(point, env);
        if drop_changed {
            self.request_rebuild();
        }
        changed |= drop_changed;
        self.text_editing.active_text_selection_drag = None;
        self.hit_test.active_pointer_drag_target = None;
        self.hit_test.active_pointer_drag_signature = None;
        self.hit_test.active_press_bounds = None;
        self.hit_test.active_press_origin = None;
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        if press_clear.chrome_changed {
            self.request_rebuild();
        } else if press_clear.visual_changed {
            self.request_redraw();
        }
        changed |= press_clear.visual_changed || press_clear.chrome_changed;
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
        let hover = self.hit_test.sync_hover_targets(point, env, true, at);
        if hover.visual_changed {
            self.request_redraw();
        }
        if hover.handler_changed {
            self.request_rebuild();
        }
        rebuild_requested |= hover.handler_changed;
        tracing::trace!(
            target: "waterui::hydrolysis::input",
            x,
            y,
            changed = rebuild_requested,
            drag_changed,
            gesture_changed,
            hover_visual = hover.visual_changed,
            dragging = self.hit_test.active_pointer_drag_target.is_some(),
            gesture_active = self.gesture_engine.has_active_recognizer(),
            "pointer move handled"
        );
        rebuild_requested || hover.visual_changed
    }

    pub fn sync_pointer_hover_state(&mut self, x: f32, y: f32, env: &Environment) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        let at = self.frame_instant();
        let hover = self.hit_test.sync_hover_targets(point, env, false, at);
        if hover.visual_changed {
            self.request_redraw();
        }
        let changed = hover.handler_changed;
        if changed {
            self.request_rebuild();
        }
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
        rebuild_requested |= self.cancel_active_drag(env);
        let press_clear = self.hit_test.interaction.clear_all_presses(at);
        if press_clear.chrome_changed {
            self.request_rebuild();
        } else if press_clear.visual_changed {
            self.request_redraw();
        }
        rebuild_requested |= press_clear.chrome_changed;
        let gesture_changed = self
            .gesture_engine
            .handle_pointer_cancel(self.frame_instant(), env);
        rebuild_requested |= gesture_changed;
        let mut hover_visual_changed = false;
        for target in &mut self.hit_test.hover_targets {
            let hovering = self.hit_test.interaction.hovering(target.slot);
            if !hovering {
                continue;
            }
            self.hit_test.interaction.set_hovering(target.slot, false);
            if let Some(handles) = &target.handles {
                handles.set_hovering(false, at);
                hover_visual_changed = true;
            }
            if let Some(on_exit) = target.on_exit.as_mut() {
                rebuild_requested |= (on_exit.borrow_mut())(env);
            }
        }
        if hover_visual_changed {
            self.request_redraw();
        }
        rebuild_requested || hover_visual_changed
    }

    pub fn handle_scroll(&mut self, x: f32, y: f32, dx: f32, dy: f32, is_line_delta: bool) -> bool {
        let point = vello::kurbo::Point::new(f64::from(x), f64::from(y));
        for target in self.hit_test.scroll_targets.iter_mut().rev() {
            if target.bounds.contains(point) {
                let changed = (target.action.borrow_mut())(dx, dy, is_line_delta);
                if changed {
                    self.dismiss_active_text_context_menu();
                    self.dismiss_active_popup_menu();
                }
                return changed;
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
        let interaction = press_slot.and_then(|slot| self.hit_test.interaction.handles_for(slot));
        self.hit_test.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag,
            depth,
            order,
            press_slot,
            interaction,
            action,
        });
    }

    pub(crate) fn register_draggable_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        data: Computed<DragData>,
    ) {
        self.register_pointer_target_action(
            bounds,
            true,
            None,
            Rc::new(RefCell::new(
                move |renderer: &mut HydrolysisRenderer,
                      point: vello::kurbo::Point,
                      env: &Environment| {
                    renderer.begin_or_update_drag(data.get(), point, env)
                },
            )),
            self.render_depth,
        );
    }

    /// Register a drop target from pre-wrapped, shareable handler handles. A
    /// retained `Wrapper` node wraps the destination's handlers once at build and
    /// re-registers the same `Rc`s on every flush (it holds the destination by
    /// reference and cannot move the handlers out each frame).
    pub(crate) fn register_drop_destination_handles(
        &mut self,
        bounds: vello::kurbo::Rect,
        handles: &DropDestinationHandles,
        env: &Environment,
    ) {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        let order = self.hit_test.next_hit_test_order();
        self.hit_test.drop_targets.push(DropTarget {
            bounds,
            key: DropTargetKey {
                depth: self.render_depth,
                order,
            },
            env: env.clone(),
            on_drop: Rc::clone(&handles.on_drop),
            on_enter: handles.on_enter.clone(),
            on_exit: handles.on_exit.clone(),
        });
    }

    pub(crate) fn bind_interaction_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.bind_interaction_target_with_focus(bounds, env, None, false)
    }

    /// Binds an interaction target for a control that supports the disabled
    /// state: a disabled control samples an at-rest [`WidgetInteractionState`]
    /// (with its `disabled` flag set) and registers no hover target, so the
    /// pointer neither hovers nor presses it.
    pub(crate) fn bind_control_interaction_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        disabled: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.bind_interaction_target_with_focus(bounds, env, None, disabled)
    }

    pub(crate) fn bind_focused_interaction_target(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        focused: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.bind_interaction_target_with_focus(
            bounds,
            env,
            Some(InteractionFocus::visible(focused)),
            false,
        )
    }

    fn bind_interaction_target_with_focus(
        &mut self,
        bounds: vello::kurbo::Rect,
        env: &Environment,
        focus: Option<InteractionFocus>,
        disabled: bool,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        let (hover_slot, hovered) = self.hit_test.interaction.bind_hover();
        if disabled {
            // No hover target is registered for a disabled control, so pointer
            // moves never sync its slot; reset it so a stale hover from before
            // the control was disabled does not resurface on re-enable.
            self.hit_test.interaction.set_hovering(hover_slot, false);
        }
        let motion = widget_theme(env).interaction_motion();
        let now = self.frame_instant();
        let (state, press_slot, handles) = self.hit_test.interaction.bind_widget_state(
            WidgetInteractionInput {
                bounds,
                hovered,
                focus,
                disabled,
            },
            &motion,
            &mut self.animation_controller,
            now,
        );
        // Every widget that binds an interaction target draws its hover/focus/press
        // state layers from the sampled state each flush, so its chrome is
        // state-dependent by construction: a press or hover change must schedule a
        // re-flush (not just a re-present of the stale scene).
        handles.mark_chrome_state_dependent();
        if !disabled && self.hit_test.hit_test_opacity > HIT_TEST_ALPHA_THRESHOLD {
            self.hit_test.hover_targets.push(HoverTarget {
                bounds,
                slot: hover_slot,
                handles: Some(Rc::clone(&handles)),
                on_enter: None,
                on_move: None,
                on_exit: None,
            });
        }
        (state, press_slot, handles)
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
        let interaction = self.hit_test.interaction.handles_for(press_slot);
        self.hit_test.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag: false,
            depth: self.render_depth,
            order,
            press_slot: Some(press_slot),
            interaction,
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
        let interaction = self.hit_test.interaction.handles_for(press_slot);
        self.hit_test.pointer_targets.push(PointerTarget {
            bounds,
            captures_drag: true,
            depth: self.render_depth,
            order,
            press_slot: Some(press_slot),
            interaction,
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
        self.register_hover_target_with_handles(bounds, None, on_enter, on_move, on_exit);
    }

    pub(crate) fn register_hover_target_with_handles(
        &mut self,
        bounds: vello::kurbo::Rect,
        handles: Option<Rc<InteractionLayerHandles>>,
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
            handles,
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

    pub(crate) fn register_scroll_target<F>(
        &mut self,
        bounds: vello::kurbo::Rect,
        handle: crate::scroll::ScrollHandle,
        action: F,
    ) where
        F: 'static + FnMut(f32, f32, bool) -> bool,
    {
        if self.hit_test.hit_test_opacity <= HIT_TEST_ALPHA_THRESHOLD {
            return;
        }
        self.hit_test.scroll_targets.push(ScrollTarget {
            bounds,
            action: Rc::new(RefCell::new(action)),
            handle,
        });
    }

    /// Advances every scroll view's smoothed wheel scroll and reports whether
    /// more animation frames are needed.
    pub(crate) fn tick_smooth_scrolls(&mut self, now: Instant) -> bool {
        let mut active = false;
        for target in &self.hit_test.scroll_targets {
            active |= target.handle.tick_smooth_scroll(now);
        }
        active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> vello::kurbo::Rect {
        vello::kurbo::Rect::new(x0, y0, x1, y1)
    }

    #[test]
    fn cursor_style_falls_back_to_arrow_outside_all_targets() {
        let state = HitTestState::default();
        assert_eq!(
            state.cursor_style_at(vello::kurbo::Point::new(5.0, 5.0)),
            CursorStyle::Arrow
        );
    }

    #[test]
    fn cursor_style_prefers_the_last_registered_target() {
        let mut state = HitTestState::default();
        state.cursor_targets.push(CursorTarget {
            bounds: rect(0.0, 0.0, 100.0, 100.0),
            style: CursorStyle::PointingHand,
        });
        state.cursor_targets.push(CursorTarget {
            bounds: rect(25.0, 25.0, 75.0, 75.0),
            style: CursorStyle::IBeam,
        });

        // Inside both: the later (topmost-drawn) target wins.
        assert_eq!(
            state.cursor_style_at(vello::kurbo::Point::new(50.0, 50.0)),
            CursorStyle::IBeam
        );
        // Inside only the first.
        assert_eq!(
            state.cursor_style_at(vello::kurbo::Point::new(10.0, 10.0)),
            CursorStyle::PointingHand
        );
    }

    #[test]
    fn hit_test_order_is_monotonic_and_resets_per_rebuild() {
        let mut state = HitTestState::default();
        assert_eq!(state.next_hit_test_order(), 0);
        assert_eq!(state.next_hit_test_order(), 1);
        // A scene reset keeps registration order monotonic (parametric
        // refreshes append targets); only a structural rebuild restarts it.
        state.reset_scene();
        assert_eq!(state.next_hit_test_order(), 2);
        state.begin_rebuild_frame();
        assert_eq!(state.next_hit_test_order(), 0);
    }
}
