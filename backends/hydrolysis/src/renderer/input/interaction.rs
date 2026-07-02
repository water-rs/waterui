use super::*;
use crate::animation::AnimationKey;
use waterui_backend_core::widget::{InteractionMotion, MAX_PRESS_WAVES, WidgetInteractionState};

const INTERACTION_FOCUS_KEY: usize = 0;
const INTERACTION_STATE_LAYER_KEY: usize = 1;
/// First animation key of the per-wave pairs; each press wave claims two
/// consecutive keys (opacity, grow progress).
const INTERACTION_WAVE_KEYS_BASE: usize = 2;
const INTERACTION_KEYS_PER_WAVE: usize = 2;
/// Renderer-local animation keys claimed by one widget's interaction slot.
const INTERACTION_KEYS_PER_SLOT: usize =
    INTERACTION_WAVE_KEYS_BASE + INTERACTION_KEYS_PER_WAVE * MAX_PRESS_WAVES;

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
            .checked_mul(INTERACTION_KEYS_PER_SLOT)
            .expect("interaction animation key overflow");
        let previous = self.press_controller.slots[press_slot.index].handles.take();
        // Flag slot reuse by an unrelated widget: the previous occupant sat at a
        // different position than the widget now binding this slot.
        let slot_reused = self.press_controller.slots[press_slot.index]
            .last_bounds
            .replace(input.bounds)
            .is_some_and(|bounds| !interaction_bounds_match(bounds, input.bounds));

        // Inherited press/hover must not migrate to a different widget: a wave
        // only survives if its press origin still lands inside the widget's
        // bounds, and any state from a reused slot (different-position
        // occupant) is dropped.
        if let Some(prev) = &previous {
            if slot_reused {
                prev.clear_press_state();
            } else {
                prev.retain_waves_with_origin(|origin| input.bounds.contains(origin));
            }
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
        let waves = core::array::from_fn(|index| {
            let wave_key_base =
                animation_key_base + INTERACTION_WAVE_KEYS_BASE + INTERACTION_KEYS_PER_WAVE * index;
            let wave_visual = previous.as_ref().is_some_and(|prev| {
                prev.wave(index)
                    .visually_pressed(motion.minimum_press_duration, now)
            });
            let alpha = animation_controller.bind_scalar_target(
                AnimationKey::renderer_local_scalar(wave_key_base),
                if wave_visual {
                    motion.pressed_opacity
                } else {
                    0.0
                },
                press_layer_opacity_animation(wave_visual, motion),
                now,
            );
            // Material ripple geometry only ever plays forward: a wave grows
            // from its press point and, once released, holds its expanded shape
            // while its layer fades out. While the layer is still visible the
            // progress therefore keeps its 1.0 target (re-targeting 0 with the
            // grow animation would play the expansion backwards — a shrinking
            // ripple); it snaps to 0 only once the fade-out has finished and
            // the wave is invisible.
            let wave_visible = wave_visual || alpha.sample(now) > 0.0;
            let progress = animation_controller.bind_scalar_target(
                AnimationKey::renderer_local_scalar(wave_key_base + 1),
                if wave_visible { 1.0 } else { 0.0 },
                if wave_visible {
                    motion.press_grow.clone()
                } else {
                    Animation::linear(Duration::ZERO)
                },
                now,
            );
            WaveLayer::new(alpha, progress)
        });

        let handles = Rc::new(InteractionLayerHandles::new(
            hover_alpha.clone(),
            waves,
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
            pressed: handles.visually_pressed(now),
            focus_visible,
            focus_progress: focus_alpha.sample(now),
            state_layer_opacity: hover_alpha.sample(now),
            press_waves: handles.sample_waves(now),
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
    let inverse = hit_transform.inverse();
    state.press_waves.map_origins(|origin| inverse * origin);
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
    use super::super::interaction_layers::{InteractionLayerHandles, WaveLayer};
    use super::{InteractionEngine, WidgetInteractionInput};
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
        let mut bind = |key: usize| {
            controller.bind_scalar_target(
                AnimationKey::renderer_local_scalar(key),
                0.0,
                Animation::linear(Duration::ZERO),
                now,
            )
        };
        let hover_alpha = bind(0);
        let waves = core::array::from_fn(|index| {
            WaveLayer::new(bind(1 + index * 2), bind(2 + index * 2))
        });
        InteractionLayerHandles::new(hover_alpha, waves, motion())
    }

    #[test]
    fn released_ripple_holds_full_size_while_fading_out() {
        let started = Instant::now();
        let mut engine = InteractionEngine::default();
        let mut controller = AnimationController::default();
        let motion = motion();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 100.0, 40.0);

        let bind = |engine: &mut InteractionEngine,
                        controller: &mut AnimationController,
                        now: Instant| {
            engine.begin_rebuild_frame();
            controller.begin_rebuild_frame();
            let bound = engine.bind_widget_state(
                WidgetInteractionInput {
                    bounds,
                    hovered: false,
                    focus: None,
                },
                &motion,
                controller,
                now,
            );
            controller.finish_rebuild_frame_with_inactive_slot_retention(false);
            engine.finish_rebuild_frame();
            bound
        };

        let (_, slot, _) = bind(&mut engine, &mut controller, started);
        engine.begin_press(slot, vello::kurbo::Point::new(10.0, 10.0), started);

        // Release after the grow completed (test grow: 225ms linear); the
        // minimum press duration (75ms) has elapsed, so the release applies
        // the fade-out (120ms linear) immediately.
        let released = started + Duration::from_millis(230);
        engine.clear_all_presses(released);

        // Rebind mid-fade: the ripple must keep its expanded shape (progress
        // stays 1.0) instead of playing the grow animation backwards.
        let fading = released + Duration::from_millis(30);
        let (state, _, _) = bind(&mut engine, &mut controller, fading);
        let wave = state
            .press_waves
            .latest()
            .expect("fading wave must still be sampled");
        assert!(wave.opacity > 0.0, "fade-out must still be in flight");
        assert!(
            (wave.progress - 1.0).abs() < f32::EPSILON,
            "ripple geometry must not shrink during the fade-out"
        );

        // Once the fade-out has finished the wave is invisible and dropped
        // from the sampled set.
        let faded = released + Duration::from_millis(160);
        let (state, _, _) = bind(&mut engine, &mut controller, faded);
        assert!(
            state.press_waves.is_empty(),
            "fully faded wave must no longer be sampled"
        );
    }

    #[test]
    fn rapid_represses_overlap_independent_waves() {
        let started = Instant::now();
        let mut engine = InteractionEngine::default();
        let mut controller = AnimationController::default();
        let motion = motion();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 100.0, 40.0);

        let bind = |engine: &mut InteractionEngine,
                    controller: &mut AnimationController,
                    now: Instant| {
            engine.begin_rebuild_frame();
            controller.begin_rebuild_frame();
            let bound = engine.bind_widget_state(
                WidgetInteractionInput {
                    bounds,
                    hovered: false,
                    focus: None,
                },
                &motion,
                controller,
                now,
            );
            controller.finish_rebuild_frame_with_inactive_slot_retention(false);
            engine.finish_rebuild_frame();
            bound
        };

        let (_, slot, _) = bind(&mut engine, &mut controller, started);
        engine.begin_press(slot, vello::kurbo::Point::new(10.0, 10.0), started);

        // Quick tap: released long before the grow (225ms) finishes.
        engine.clear_all_presses(started + Duration::from_millis(40));

        // Re-press at a different point while the first wave is mid-flight.
        let repressed = started + Duration::from_millis(100);
        let (_, slot, _) = bind(&mut engine, &mut controller, repressed);
        engine.begin_press(slot, vello::kurbo::Point::new(80.0, 30.0), repressed);

        // Both waves are visible: the released first wave keeps its own grow
        // progress and origin while the fresh wave starts over from zero.
        let sampled_at = repressed + Duration::from_millis(10);
        let (state, _, _) = bind(&mut engine, &mut controller, sampled_at);
        let waves: Vec<_> = state.press_waves.iter().collect();
        assert_eq!(waves.len(), 2, "both press waves must be visible");
        assert_eq!(
            waves[0].origin,
            Some(vello::kurbo::Point::new(10.0, 10.0)),
            "the older wave keeps the first press origin"
        );
        assert_eq!(
            waves[1].origin,
            Some(vello::kurbo::Point::new(80.0, 30.0)),
            "the newest wave grows from the second press origin"
        );
        assert!(
            waves[0].progress > waves[1].progress,
            "the older wave must be further into its growth than the fresh wave"
        );

        // The first wave's fade-out (applied at the 100ms rebind once its
        // 75ms minimum press elapsed, 120ms long) must not touch the second,
        // still-pressed wave.
        let first_faded = started + Duration::from_millis(240);
        let (state, _, _) = bind(&mut engine, &mut controller, first_faded);
        let waves: Vec<_> = state.press_waves.iter().collect();
        assert_eq!(waves.len(), 1, "the first wave must have faded out alone");
        assert_eq!(
            waves[0].origin,
            Some(vello::kurbo::Point::new(80.0, 30.0)),
            "the held second wave must survive"
        );
        assert!(
            waves[0].opacity > 0.0,
            "the held second wave must stay visible"
        );
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
