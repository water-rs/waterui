use super::*;
use crate::animation::AnimationKey;
use std::collections::{BTreeMap, BTreeSet};
use waterui_backend_core::widget::{InteractionMotion, MAX_PRESS_WAVES, WidgetInteractionState};

const INTERACTION_FOCUS_KEY: usize = 0;
const INTERACTION_STATE_LAYER_KEY: usize = 1;
/// First animation key of the per-wave pairs; each press wave claims two
/// consecutive keys (opacity, grow progress).
const INTERACTION_WAVE_KEYS_BASE: usize = 2;
const INTERACTION_KEYS_PER_WAVE: usize = 2;
/// Renderer-local animation keys claimed by one semantic interaction identity.
const INTERACTION_KEYS_PER_IDENTITY: usize =
    INTERACTION_WAVE_KEYS_BASE + INTERACTION_KEYS_PER_WAVE * MAX_PRESS_WAVES;

#[derive(Debug, Default)]
pub(crate) struct InteractionEngine {
    states: BTreeMap<InteractionKey, InteractionState>,
    active: BTreeSet<InteractionKey>,
}

/// Stable identity of one semantic interaction target.
///
/// `owner` is the retained node/state `Rc`, while `discriminator` distinguishes
/// multiple controls owned by that node (for example a stepper's minus/plus
/// buttons). Identity never depends on render order or body call position.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct InteractionKey {
    owner: RetainedIdentity,
    discriminator: usize,
}

impl InteractionKey {
    pub(crate) fn for_rc<T: 'static>(owner: &Rc<T>, discriminator: usize) -> Self {
        Self {
            owner: RetainedIdentity::for_rc(owner),
            discriminator,
        }
    }

    fn animation_discriminator(&self, key: usize) -> usize {
        self.discriminator
            .checked_mul(INTERACTION_KEYS_PER_IDENTITY)
            .and_then(|base| base.checked_add(key))
            .expect("interaction animation discriminator overflow")
    }
}

#[derive(Debug, Default)]
struct InteractionState {
    hovering: bool,
    handles: Option<Rc<InteractionLayerHandles>>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InteractionFocus {
    pub(crate) visible: bool,
}

pub(crate) struct WidgetInteractionInput {
    pub(crate) bounds: vello::kurbo::Rect,
    pub(crate) hovered: bool,
    pub(crate) focus: Option<InteractionFocus>,
    /// The widget is disabled: inherited hover/press state is dropped and the
    /// sampled state stays at rest until the widget is enabled again.
    pub(crate) disabled: bool,
}

impl InteractionFocus {
    pub(crate) const fn visible(visible: bool) -> Self {
        Self { visible }
    }
}

impl InteractionEngine {
    pub(crate) fn begin_rebuild_frame(&mut self) {
        self.active.clear();
    }

    pub(crate) fn finish_rebuild_frame(&mut self) {
        self.states.retain(|key, _| self.active.contains(key));
    }

    pub(crate) fn bind_hover(&mut self, key: &InteractionKey) -> (HoverSlot, bool) {
        self.active.insert(key.clone());
        let hovering = self.states.entry(key.clone()).or_default().hovering;
        (HoverSlot { key: key.clone() }, hovering)
    }

    pub(crate) fn set_hovering(&mut self, slot: &HoverSlot, hovering: bool) {
        self.states
            .get_mut(&slot.key)
            .expect("hover target identity must be active")
            .hovering = hovering;
    }

    pub(crate) fn hovering(&self, slot: &HoverSlot) -> bool {
        self.states
            .get(&slot.key)
            .expect("hover target identity must be active")
            .hovering
    }

    pub(crate) fn begin_press(
        &mut self,
        slot: &PressSlot,
        origin: vello::kurbo::Point,
        now: Instant,
    ) {
        if let Some(handles) = self
            .states
            .get(&slot.key)
            .and_then(|state| state.handles.as_ref())
        {
            handles.begin_press(origin, now);
        }
    }

    pub(crate) fn clear_all_presses(&mut self, now: Instant) -> PressClear {
        let mut clear = PressClear::default();
        for state in self.states.values() {
            if let Some(handles) = &state.handles
                && handles.release(now)
            {
                clear.visual_changed = true;
                clear.chrome_changed |= handles.chrome_state_dependent();
            }
        }
        clear
    }

    pub(crate) fn handles_for(&self, slot: &PressSlot) -> Option<Rc<InteractionLayerHandles>> {
        self.states
            .get(&slot.key)
            .and_then(|state| state.handles.clone())
    }

    pub(crate) fn bind_widget_state(
        &mut self,
        key: &InteractionKey,
        input: WidgetInteractionInput,
        motion: &InteractionMotion,
        animation_controller: &mut AnimationController,
        now: Instant,
    ) -> (
        WidgetInteractionState,
        PressSlot,
        Rc<InteractionLayerHandles>,
    ) {
        self.active.insert(key.clone());
        let interaction_state = self.states.entry(key.clone()).or_default();
        let press_slot = PressSlot {
            key: key.clone(),
            modal: false,
            focus_binding: None,
            escape_action: None,
        };
        let previous = interaction_state.handles.take();

        // Inherited press/hover must not migrate to a different widget: a wave
        // only survives if its press origin still lands inside the widget's
        // bounds, and any state from a reused slot (different-position
        // occupant) is dropped. A disabled widget drops everything — a control
        // disabled mid-hover or mid-press comes to rest immediately.
        if let Some(prev) = &previous {
            if input.disabled {
                prev.clear_press_state();
            } else {
                prev.retain_waves_with_origin(|origin| input.bounds.contains(origin));
            }
        }
        let hovered = if input.disabled {
            false
        } else {
            previous
                .as_ref()
                .map_or(input.hovered, |prev| prev.hovering())
        };
        let focus_visible = !input.disabled && input.focus.is_some_and(|focus| focus.visible);

        let focus_alpha = animation_controller.bind_scalar_target(
            AnimationKey::renderer_local_scalar_with_discriminator(
                key.owner.address(),
                key.animation_discriminator(INTERACTION_FOCUS_KEY),
            ),
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
            AnimationKey::renderer_local_scalar_with_discriminator(
                key.owner.address(),
                key.animation_discriminator(INTERACTION_STATE_LAYER_KEY),
            ),
            hover_target,
            state_layer_animation(hover_target, motion),
            now,
        );
        let waves = core::array::from_fn(|index| {
            let wave_key_base = INTERACTION_WAVE_KEYS_BASE + INTERACTION_KEYS_PER_WAVE * index;
            let wave_visual = previous.as_ref().is_some_and(|prev| {
                prev.wave(index)
                    .visually_pressed(motion.minimum_press_duration, now)
            });
            let alpha = animation_controller.bind_scalar_target(
                AnimationKey::renderer_local_scalar_with_discriminator(
                    key.owner.address(),
                    key.animation_discriminator(wave_key_base),
                ),
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
                AnimationKey::renderer_local_scalar_with_discriminator(
                    key.owner.address(),
                    key.animation_discriminator(wave_key_base + 1),
                ),
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
        if let Some(previous) = previous.filter(|_| !input.disabled) {
            handles.copy_interaction_state_from(&previous);
        } else {
            handles.set_initial_hovering(hovered);
        }
        interaction_state.handles = Some(Rc::clone(&handles));

        let state = WidgetInteractionState {
            disabled: input.disabled,
            hovered,
            // Chrome reads the PHYSICAL press (mdui removes [pressed] the
            // instant the pointer lifts, so the 28dp pressed thumb and the
            // pressed tint drop immediately on release). The ripple's Material
            // minimum-press gating lives in the waves themselves and must not
            // leak into pressed chrome after release.
            pressed: handles.pressing(),
            focus_visible,
            focus_progress: focus_alpha.sample(now),
            state_layer_opacity: hover_alpha.sample(now),
            press_waves: handles.sample_waves(now),
        };
        (state, press_slot, handles)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PressSlot {
    pub(crate) key: InteractionKey,
    pub(crate) modal: bool,
    pub(crate) focus_binding: Option<Binding<bool>>,
    pub(crate) escape_action: Option<SharedAction>,
}

/// Outcome of releasing all active presses: `visual_changed` replays state
/// layers, `chrome_changed` means a pressed widget's chrome samples
/// interaction state and must re-render.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PressClear {
    pub(crate) visual_changed: bool,
    pub(crate) chrome_changed: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct HoverSlot {
    pub(crate) key: InteractionKey,
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
    use super::{InteractionEngine, InteractionKey, WidgetInteractionInput};
    use crate::animation::{AnimationController, AnimationKey};
    use crate::time::Instant;
    use core::time::Duration;
    use std::rc::Rc;
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
        let waves =
            core::array::from_fn(|index| WaveLayer::new(bind(1 + index * 2), bind(2 + index * 2)));
        InteractionLayerHandles::new(hover_alpha, waves, motion())
    }

    #[test]
    fn released_ripple_holds_full_size_while_fading_out() {
        let started = Instant::now();
        let mut engine = InteractionEngine::default();
        let mut controller = AnimationController::default();
        let motion = motion();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 100.0, 40.0);
        let owner = Rc::new(());
        let key = InteractionKey::for_rc(&owner, 0);

        let bind =
            |engine: &mut InteractionEngine, controller: &mut AnimationController, now: Instant| {
                engine.begin_rebuild_frame();
                controller.begin_rebuild_frame();
                let bound = engine.bind_widget_state(
                    &key,
                    WidgetInteractionInput {
                        bounds,
                        hovered: false,
                        focus: None,
                        disabled: false,
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
        engine.begin_press(&slot, vello::kurbo::Point::new(10.0, 10.0), started);

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
        let owner = Rc::new(());
        let key = InteractionKey::for_rc(&owner, 0);

        let bind =
            |engine: &mut InteractionEngine, controller: &mut AnimationController, now: Instant| {
                engine.begin_rebuild_frame();
                controller.begin_rebuild_frame();
                let bound = engine.bind_widget_state(
                    &key,
                    WidgetInteractionInput {
                        bounds,
                        hovered: false,
                        focus: None,
                        disabled: false,
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
        engine.begin_press(&slot, vello::kurbo::Point::new(10.0, 10.0), started);

        // Quick tap: released long before the grow (225ms) finishes.
        engine.clear_all_presses(started + Duration::from_millis(40));

        // Re-press at a different point while the first wave is mid-flight.
        let repressed = started + Duration::from_millis(100);
        let (_, slot, _) = bind(&mut engine, &mut controller, repressed);
        engine.begin_press(&slot, vello::kurbo::Point::new(80.0, 30.0), repressed);

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
    fn disabled_widget_drops_press_and_hover_and_samples_at_rest() {
        let started = Instant::now();
        let mut engine = InteractionEngine::default();
        let mut controller = AnimationController::default();
        let motion = motion();
        let bounds = vello::kurbo::Rect::new(0.0, 0.0, 100.0, 40.0);
        let owner = Rc::new(());
        let key = InteractionKey::for_rc(&owner, 0);

        let bind = |engine: &mut InteractionEngine,
                    controller: &mut AnimationController,
                    now: Instant,
                    hovered: bool,
                    disabled: bool| {
            engine.begin_rebuild_frame();
            controller.begin_rebuild_frame();
            let bound = engine.bind_widget_state(
                &key,
                WidgetInteractionInput {
                    bounds,
                    hovered,
                    focus: None,
                    disabled,
                },
                &motion,
                controller,
                now,
            );
            controller.finish_rebuild_frame_with_inactive_slot_retention(false);
            engine.finish_rebuild_frame();
            bound
        };

        // Hovered and mid-press, then the widget becomes disabled: the
        // sampled state comes to rest immediately and carries the flag.
        let (_, slot, _) = bind(&mut engine, &mut controller, started, true, false);
        engine.begin_press(&slot, vello::kurbo::Point::new(10.0, 10.0), started);

        let disabled_at = started + Duration::from_millis(50);
        let (state, _, _) = bind(&mut engine, &mut controller, disabled_at, true, true);
        assert!(state.disabled);
        assert!(!state.hovered, "disabled widget must not sample hover");
        assert!(!state.pressed, "disabled widget must not sample press");

        // The in-flight ripple is released, fades out (mdui keeps the fade),
        // and must be gone once the fade-out has finished.
        let faded_at = disabled_at + Duration::from_millis(200);
        let (state, _, _) = bind(&mut engine, &mut controller, faded_at, true, true);
        assert!(
            state.press_waves.is_empty(),
            "the released ripple must finish fading while disabled"
        );

        // Re-enabling starts at rest: the stale press must not resurface.
        let reenabled_at = faded_at + Duration::from_millis(50);
        let (state, _, _) = bind(&mut engine, &mut controller, reenabled_at, false, false);
        assert!(!state.disabled);
        assert!(!state.pressed);
        assert!(state.press_waves.is_empty());
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
