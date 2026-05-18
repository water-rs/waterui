use super::*;
use waterui_backend_core::widget::{InteractionMotion, WidgetInteractionState};

#[derive(Debug, Default)]
pub(crate) struct InteractionEngine {
    hover_controller: HoverController,
    press_controller: PressController,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InteractionFocus {
    visible: bool,
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

    pub(crate) fn clear_all_presses(&mut self, now: Instant) -> bool {
        self.press_controller.clear_all(now)
    }

    pub(crate) fn bind_widget_state(
        &mut self,
        bounds: vello::kurbo::Rect,
        hovered: bool,
        focus: Option<InteractionFocus>,
        active_press_origin: Option<vello::kurbo::Point>,
        motion: &InteractionMotion,
        animation_controller: &mut AnimationController,
        now: Instant,
    ) -> (WidgetInteractionState, PressSlot) {
        let focus_progress = focus.map_or(0.0, |focus| {
            animation_controller
                .bind_scalar_target(
                    if focus.visible { 1.0 } else { 0.0 },
                    if focus.visible {
                        motion.focus_enter.clone()
                    } else {
                        motion.focus_exit.clone()
                    },
                    now,
                )
                .sample(now)
        });
        let (press_slot, slot_pressed) = self.press_controller.bind();
        let active_bounds_pressed =
            active_press_origin.is_some_and(|origin| bounds.contains(origin));
        let slot = &mut self.press_controller.slots[press_slot.index];
        let slot_origin_in_bounds = slot.origin.is_some_and(|origin| bounds.contains(origin));
        let slot_pressed = slot_pressed && slot_origin_in_bounds;
        let released_press_visible =
            slot_origin_in_bounds && released_before_minimum_press_duration(slot, now, motion);
        let visual_pressed = slot_pressed || active_bounds_pressed || released_press_visible;
        if should_clear_released_press_origin(slot, now, motion) {
            slot.origin = None;
            slot.pressed_at = None;
            slot.released_at = None;
        }
        let press_origin = if active_bounds_pressed {
            active_press_origin.or(slot.origin)
        } else if slot_origin_in_bounds {
            slot.origin
        } else {
            None
        };
        let target_opacity = if hovered { motion.hover_opacity } else { 0.0 };
        let alpha_handle = animation_controller.bind_scalar_target(
            target_opacity,
            state_layer_animation(target_opacity, motion),
            now,
        );
        let state_layer_opacity = alpha_handle.sample(now);
        let press_opacity_handle = animation_controller.bind_scalar_target(
            if visual_pressed {
                motion.pressed_opacity
            } else {
                0.0
            },
            press_layer_opacity_animation(visual_pressed, motion),
            now,
        );
        let press_layer_opacity = press_opacity_handle.sample(now);
        let press_progress_handle = animation_controller.bind_scalar_target(
            if press_origin.is_some() { 1.0 } else { 0.0 },
            motion.press_grow.clone(),
            now,
        );
        let press_progress = press_progress_handle.sample(now);
        (
            WidgetInteractionState {
                hovered,
                pressed: visual_pressed,
                focus_visible: focus.is_some_and(|focus| focus.visible),
                focus_progress,
                state_layer_opacity,
                press_layer_opacity,
                press_origin,
                press_progress,
            },
            press_slot,
        )
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

#[derive(Debug)]
pub(crate) struct PressStateSlot {
    pub(crate) pressing: bool,
    pub(crate) origin: Option<vello::kurbo::Point>,
    pub(crate) pressed_at: Option<Instant>,
    pub(crate) released_at: Option<Instant>,
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
            self.slots.push(PressStateSlot {
                pressing: false,
                origin: None,
                pressed_at: None,
                released_at: None,
            });
        }
        (PressSlot { index }, self.slots[index].pressing)
    }

    pub(crate) fn begin_press(
        &mut self,
        slot: PressSlot,
        origin: vello::kurbo::Point,
        now: Instant,
    ) {
        let state = &mut self.slots[slot.index];
        state.pressing = true;
        state.origin = Some(origin);
        state.pressed_at = Some(now);
        state.released_at = None;
    }

    pub(crate) fn clear_all(&mut self, now: Instant) -> bool {
        let mut changed = false;
        for slot in &mut self.slots {
            changed |= slot.pressing;
            slot.pressing = false;
            if slot.origin.is_some() && slot.released_at.is_none() {
                slot.released_at = Some(now);
            }
        }
        changed
    }
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

pub(crate) fn released_before_minimum_press_duration(
    slot: &PressStateSlot,
    now: Instant,
    motion: &InteractionMotion,
) -> bool {
    if slot.pressing || slot.released_at.is_none() {
        return false;
    }
    let Some(pressed_at) = slot.pressed_at else {
        return false;
    };
    now.duration_since(pressed_at) < motion.minimum_press_duration
}

pub(crate) fn should_clear_released_press_origin(
    slot: &PressStateSlot,
    now: Instant,
    motion: &InteractionMotion,
) -> bool {
    if slot.pressing {
        return false;
    }
    let Some(released_at) = slot.released_at else {
        return false;
    };
    let minimum_remaining = slot.pressed_at.map_or(Duration::ZERO, |pressed_at| {
        motion
            .minimum_press_duration
            .saturating_sub(now.duration_since(pressed_at))
    });
    let retention = motion.press_fade_out.duration().max(minimum_remaining);
    now.duration_since(released_at) >= retention
}

#[cfg(test)]
mod tests {
    use super::{
        PressStateSlot, released_before_minimum_press_duration, should_clear_released_press_origin,
    };
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

    #[test]
    fn released_press_stays_visually_pressed_until_minimum_duration() {
        let started = Instant::now();
        let released = started + Duration::from_millis(10);
        let slot = PressStateSlot {
            pressing: false,
            origin: Some(vello::kurbo::Point::new(4.0, 5.0)),
            pressed_at: Some(started),
            released_at: Some(released),
        };

        assert!(released_before_minimum_press_duration(
            &slot,
            started + Duration::from_millis(20),
            &motion()
        ));
        assert!(!released_before_minimum_press_duration(
            &slot,
            started + Duration::from_millis(80),
            &motion()
        ));
    }

    #[test]
    fn released_press_origin_survives_fade_out_then_clears() {
        let started = Instant::now();
        let released = started + Duration::from_millis(10);
        let slot = PressStateSlot {
            pressing: false,
            origin: Some(vello::kurbo::Point::new(4.0, 5.0)),
            pressed_at: Some(started),
            released_at: Some(released),
        };

        assert!(!should_clear_released_press_origin(
            &slot,
            released + Duration::from_millis(70),
            &motion()
        ));
        assert!(should_clear_released_press_origin(
            &slot,
            released + Duration::from_millis(130),
            &motion()
        ));
    }
}
