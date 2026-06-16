use super::*;
use crate::animation::AnimationKey;
use waterui_backend_core::widget::{InteractionMotion, WidgetInteractionState};

const INTERACTION_FOCUS_KEY: usize = 0;
const INTERACTION_STATE_LAYER_KEY: usize = 1;
const INTERACTION_PRESS_OPACITY_KEY: usize = 2;
const INTERACTION_PRESS_PROGRESS_KEY: usize = 3;

#[derive(Debug, Default)]
pub(crate) struct InteractionEngine {
    hover_controller: HoverController,
    press_controller: PressController,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InteractionFocus {
    visible: bool,
}

pub(crate) struct WidgetInteractionInput {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) hovered: bool,
    pub(crate) focus: Option<InteractionFocus>,
    pub(crate) active_press_origin: Option<vello::kurbo::Point>,
}

impl InteractionFocus {
    pub(crate) const fn visible(visible: bool) -> Self {
        Self { visible }
    }
}

impl InteractionEngine {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.hover_controller.begin_rebuild_frame();
        self.press_controller.begin_rebuild_frame();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.hover_controller.finish_rebuild_frame();
        self.press_controller.finish_rebuild_frame();
    }

    pub(crate) fn bind_hover(&mut self) -> (HoverSlot, bool) {
        self.hover_controller.bind()
    }

    pub(crate) fn hover_cursor(&self) -> usize {
        self.hover_controller.cursor()
    }

    pub(crate) fn rewind_hover_to(&mut self, cursor: usize) {
        self.hover_controller.rewind_to(cursor);
    }

    pub(crate) fn swap_hover_controller(&mut self, other: &mut HoverController) {
        core::mem::swap(&mut self.hover_controller, other);
    }

    pub(crate) fn bind_press_slot(&mut self) -> PressSlot {
        let (slot, _) = self.press_controller.bind();
        slot
    }

    pub(crate) fn set_hovering(&mut self, slot: HoverSlot, hovering: bool) {
        self.hover_controller.set_hovering(slot, hovering);
    }

    pub(crate) fn hovering(&self, slot: HoverSlot) -> bool {
        self.hover_controller.hovering(slot)
    }

    pub(crate) fn begin_press(
        &mut self,
        slot: PressSlot,
        origin: vello::kurbo::Point,
        now: Instant,
    ) {
        self.press_controller.begin_press(slot, origin, now);
    }

    pub(crate) fn clear_all_presses(&mut self, now: Instant) -> PressClear {
        self.press_controller.clear_all(now)
    }

    pub(crate) fn handles_for(&self, slot: PressSlot) -> Option<Rc<InteractionLayerHandles>> {
        self.press_controller.slots[slot.index].handles.clone()
    }

    pub(crate) fn attach_handles(&mut self, slot: PressSlot, handles: Rc<InteractionLayerHandles>) {
        self.press_controller.slots[slot.index].handles = Some(handles);
    }

    pub(crate) fn bind_widget_state(
        &mut self,
        input: WidgetInteractionInput,
        motion: &InteractionMotion,
        animation_controller: &mut AnimationController,
        now: Instant,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        let (press_slot, _) = self.press_controller.bind();
        let animation_key_base = press_slot
            .index
            .checked_mul(4)
            .expect("interaction animation key overflow");
        let previous = self.press_controller.slots[press_slot.index].handles.take();
        // Flag slot reuse by an unrelated widget: the previous occupant sat at a
        // different position than the widget now binding this slot.
        let slot_reused = self.press_controller.slots[press_slot.index]
            .last_bounds
            .replace(input.bounds)
            .is_some_and(|bounds| !interaction_bounds_match(bounds, input.bounds));

        // Inherited press/hover must not migrate to a different widget: a press
        // only survives if its origin still lands inside the widget's bounds, and
        // any state from a reused slot (different-position occupant) is dropped.
        if let Some(prev) = &previous
            && (slot_reused
                || prev
                    .origin_in_window()
                    .is_none_or(|origin| !input.bounds.contains(origin)))
        {
            prev.clear_press_state();
        }
        let hovered = if slot_reused {
            input.hovered
        } else {
            previous
                .as_ref()
                .map_or(input.hovered, |prev| prev.hovering())
        };
        let focus_visible = input.focus.is_some_and(|focus| focus.visible);

        let focus_alpha = animation_controller.bind_scalar_target(
            AnimationKey::renderer_local_scalar(animation_key_base + INTERACTION_FOCUS_KEY),
            if focus_visible { 1.0 } else { 0.0 },
            if focus_visible {
                motion.focus_enter.clone()
            } else {
                motion.focus_exit.clone()
            },
            now,
        );
        let hover_target = if hovered { motion.hover_opacity } else { 0.0 };
        let hover_alpha = animation_controller.bind_scalar_target(
            AnimationKey::renderer_local_scalar(animation_key_base + INTERACTION_STATE_LAYER_KEY),
            hover_target,
            state_layer_animation(hover_target, motion),
            now,
        );
        let visual_pressed = !slot_reused
            && previous
                .as_ref()
                .is_some_and(|prev| prev.visually_pressed(now));
        let press_alpha = animation_controller.bind_scalar_target(
            AnimationKey::renderer_local_scalar(animation_key_base + INTERACTION_PRESS_OPACITY_KEY),
            if visual_pressed {
                motion.pressed_opacity
            } else {
                0.0
            },
            press_layer_opacity_animation(visual_pressed, motion),
            now,
        );
        let press_progress_handle = animation_controller.bind_scalar_target(
            AnimationKey::renderer_local_scalar(
                animation_key_base + INTERACTION_PRESS_PROGRESS_KEY,
            ),
            if visual_pressed { 1.0 } else { 0.0 },
            motion.press_grow.clone(),
            now,
        );

        let handles = Rc::new(InteractionLayerHandles::new(
            hover_alpha.clone(),
            press_alpha.clone(),
            press_progress_handle.clone(),
            focus_alpha.clone(),
            motion.clone(),
        ));
        // A reused slot's previous state belongs to a different widget; start fresh.
        if let Some(previous) = previous.filter(|_| !slot_reused) {
            handles.copy_interaction_state_from(&previous);
        } else {
            handles.set_initial_hovering(input.hovered);
        }
        self.press_controller.slots[press_slot.index].handles = Some(Rc::clone(&handles));

        let state = WidgetInteractionState {
            hovered,
            pressed: visual_pressed,
            focus_visible,
            focus_progress: focus_alpha.sample(now),
            state_layer_opacity: hover_alpha.sample(now),
            press_layer_opacity: press_alpha.sample(now),
            press_origin: handles.origin_in_window(),
            press_progress: press_progress_handle.sample(now),
        };
        (state, press_slot, handles)
    }
}

#[derive(Debug, Default)]
pub(crate) struct PressController {
    pub(crate) slots: Vec<PressStateSlot>,
    pub(crate) cursor: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct PressSlot {
    pub(crate) index: usize,
}

/// Per-widget press slot: the cursor allocates stable renderer-local
/// animation key indices, while the interaction state itself lives in the
/// shared [`InteractionLayerHandles`].
#[derive(Debug, Default)]
pub(crate) struct PressStateSlot {
    pub(crate) handles: Option<Rc<InteractionLayerHandles>>,
    /// The window-space bounds of the widget that last occupied this slot. A
    /// cursor-allocated slot index can be reassigned to a *different* widget when
    /// an earlier collection's membership changes (its item count shifts every
    /// later widget's slot); a bounds mismatch flags that reuse so inherited
    /// hover/press state is not migrated to the wrong widget.
    pub(crate) last_bounds: Option<vello::kurbo::Rect>,
}

/// Whether two window-space widget bounds are close enough to be the same widget
/// across frames (position and size unchanged within half a pixel).
fn interaction_bounds_match(a: vello::kurbo::Rect, b: vello::kurbo::Rect) -> bool {
    const EPSILON: f64 = 0.5;
    (a.x0 - b.x0).abs() <= EPSILON
        && (a.y0 - b.y0).abs() <= EPSILON
        && (a.x1 - b.x1).abs() <= EPSILON
        && (a.y1 - b.y1).abs() <= EPSILON
}

impl PressController {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    pub(crate) fn bind(&mut self) -> (PressSlot, bool) {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("press controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(PressStateSlot::default());
        }
        let pressing = self.slots[index]
            .handles
            .as_ref()
            .is_some_and(|handles| handles.pressing());
        (PressSlot { index }, pressing)
    }

    pub(crate) fn begin_press(
        &mut self,
        slot: PressSlot,
        origin: vello::kurbo::Point,
        now: Instant,
    ) {
        if let Some(handles) = &self.slots[slot.index].handles {
            handles.begin_press(origin, now);
        }
    }

    pub(crate) fn clear_all(&mut self, now: Instant) -> PressClear {
        let mut clear = PressClear::default();
        for slot in &self.slots {
            if let Some(handles) = &slot.handles
                && handles.release(now)
            {
                clear.visual_changed = true;
                clear.chrome_changed |= handles.chrome_state_dependent();
            }
        }
        clear
    }
}

/// Outcome of releasing all active presses: `visual_changed` replays state
/// layers, `chrome_changed` means a pressed widget's chrome samples
/// interaction state and must re-render.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PressClear {
    pub(crate) visual_changed: bool,
    pub(crate) chrome_changed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct HoverController {
    pub(crate) slots: Vec<HoverStateSlot>,
    pub(crate) cursor: usize,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct HoverSlot {
    pub(crate) index: usize,
}

#[derive(Debug)]
pub(crate) struct HoverStateSlot {
    pub(crate) hovering: bool,
}

impl HoverController {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.cursor = 0;
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.slots.truncate(self.cursor);
    }

    pub(crate) fn bind(&mut self) -> (HoverSlot, bool) {
        let index = self.cursor;
        self.cursor = self
            .cursor
            .checked_add(1)
            .expect("hover controller cursor overflow");
        if index == self.slots.len() {
            self.slots.push(HoverStateSlot { hovering: false });
        }
        (HoverSlot { index }, self.slots[index].hovering)
    }

    pub(crate) fn hovering(&self, slot: HoverSlot) -> bool {
        self.slots[slot.index].hovering
    }

    pub(crate) fn set_hovering(&mut self, slot: HoverSlot, hovering: bool) {
        self.slots[slot.index].hovering = hovering;
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(crate) fn rewind_to(&mut self, cursor: usize) {
        assert!(
            (cursor <= self.cursor),
            "hover controller rewind cursor exceeds current cursor"
        );
        self.cursor = cursor;
        self.slots.truncate(cursor);
    }
}

pub(crate) fn local_interaction_state(
    mut state: WidgetInteractionState,
    hit_transform: vello::kurbo::Affine,
) -> WidgetInteractionState {
    state.press_origin = state
        .press_origin
        .map(|origin| hit_transform.inverse() * origin);
    state
}

fn state_layer_animation(target_opacity: f32, motion: &InteractionMotion) -> Animation {
    if target_opacity > 0.0 {
        motion.hover_enter.clone()
    } else {
        motion.hover_exit.clone()
    }
}

fn press_layer_opacity_animation(pressed: bool, motion: &InteractionMotion) -> Animation {
    if pressed {
        motion.press_fade_in.clone()
    } else {
        motion.press_fade_out.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::super::interaction_layers::InteractionLayerHandles;
    use crate::animation::{AnimationController, AnimationKey};
    use crate::time::Instant;
    use core::time::Duration;
    use waterui::animation::Animation;
    use waterui_backend_core::widget::InteractionMotion;

    fn motion() -> InteractionMotion {
        InteractionMotion {
            hover_opacity: 0.08,
            focus_opacity: 0.12,
            pressed_opacity: 0.12,
            dragged_opacity: 0.16,
            hover_enter: Animation::linear(Duration::from_millis(15)),
            hover_exit: Animation::linear(Duration::from_millis(15)),
            focus_enter: Animation::linear(Duration::from_millis(15)),
            focus_exit: Animation::linear(Duration::from_millis(15)),
            press_fade_in: Animation::linear(Duration::from_millis(15)),
            press_fade_out: Animation::linear(Duration::from_millis(120)),
            press_grow: Animation::linear(Duration::from_millis(225)),
            minimum_press_duration: Duration::from_millis(75),
            touch_delay: Duration::from_millis(0),
        }
    }

    fn handles(now: Instant) -> InteractionLayerHandles {
        let mut controller = AnimationController::default();
        let bind = |controller: &mut AnimationController, key: usize| {
            controller.bind_scalar_target(
                AnimationKey::renderer_local_scalar(key),
                0.0,
                Animation::linear(Duration::ZERO),
                now,
            )
        };
        InteractionLayerHandles::new(
            bind(&mut controller, 0),
            bind(&mut controller, 1),
            bind(&mut controller, 2),
            bind(&mut controller, 3),
            motion(),
        )
    }

    #[test]
    fn released_press_stays_visually_pressed_until_minimum_duration() {
        let started = Instant::now();
        let handles = handles(started);
        handles.begin_press(vello::kurbo::Point::new(4.0, 5.0), started);
        assert!(handles.release(started + Duration::from_millis(10)));

        assert!(handles.visually_pressed(started + Duration::from_millis(20)));
        assert!(handles.flush_release(started + Duration::from_millis(20)));
        assert!(!handles.flush_release(started + Duration::from_millis(80)));
        assert!(!handles.visually_pressed(started + Duration::from_millis(80)));
    }
}
