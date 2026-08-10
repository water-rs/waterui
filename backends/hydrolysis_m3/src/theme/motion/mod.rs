//! Material Design 3 component motion.
//!
//! Every timing here is either a token from [`tokens`] or a named constant that
//! documents why it sits off the scale. Reading a component's motion should
//! answer "which M3 token is this?" without having to recognise a number.

pub mod tokens;

use core::time::Duration;

use waterui::animation::Animation;
use waterui::reactive::watcher::Context;
use waterui::{Computed, Signal};
use waterui_backend_core::widget::{
    InteractionMotion, NavigationMotion, ProgressMotion, RadioSelectionMotion, TextCaretMotion,
};

use super::dimensions::NAVIGATION_SHARED_AXIS_SLIDE_DISTANCE;
use tokens::{duration, easing, motion};

/// The MD3 standard easing curve over `duration`.
const fn material_standard(duration: Duration) -> Animation {
    motion(easing::STANDARD, duration)
}

/// The ripple animates on timings local to the component rather than on the M3
/// duration scale (`mdui/packages/mdui/src/components/ripple/style.less`): the
/// wave grows over 225ms, its opacity fades in over 75ms, and the state-layer
/// background cross-fades over 280ms. Only the release fade-out lands on a
/// token. These are named here so they are visibly deliberate rather than
/// mistaken for tokens.
const RIPPLE_RADIUS_IN: Duration = Duration::from_millis(225);
const RIPPLE_OPACITY_IN: Duration = Duration::from_millis(75);
const RIPPLE_OPACITY_OUT: Duration = duration::SHORT_3;
const RIPPLE_STATE_LAYER_FADE: Duration = Duration::from_millis(280);
/// Touch presses wait this long before showing a ripple, so a scroll that
/// starts under the finger does not flash one (mdui's ripple mixin).
const RIPPLE_TOUCH_DELAY: Duration = Duration::from_millis(70);

/// MD3 interaction state-layer motion, matching the mdui reference: the ripple
/// grows from the press point with standard easing while fading in linearly,
/// and a release never plays the growth backwards — the wave holds its expanded
/// shape and only fades out once the expansion has completed, which is why
/// `minimum_press_duration` equals the grow duration.
pub const fn interaction() -> InteractionMotion {
    InteractionMotion {
        hover_opacity: 0.08,
        focus_opacity: 0.12,
        pressed_opacity: 0.12,
        dragged_opacity: 0.16,
        hover_enter: material_standard(RIPPLE_STATE_LAYER_FADE),
        hover_exit: material_standard(RIPPLE_STATE_LAYER_FADE),
        focus_enter: material_standard(RIPPLE_STATE_LAYER_FADE),
        focus_exit: material_standard(RIPPLE_STATE_LAYER_FADE),
        press_fade_in: Animation::linear(RIPPLE_OPACITY_IN),
        press_fade_out: Animation::linear(RIPPLE_OPACITY_OUT),
        press_grow: material_standard(RIPPLE_RADIUS_IN),
        minimum_press_duration: RIPPLE_RADIUS_IN,
        touch_delay: RIPPLE_TOUCH_DELAY,
    }
}

/// material-web drives both progress indicators with CSS `ease-in-out`-family
/// curves of its own rather than M3 easing tokens, and the indeterminate cycles
/// are keyframe loop lengths, not durations from the scale.
const PROGRESS_LINEAR_DETERMINATE: (f32, f32, f32, f32) = (0.4, 0.0, 0.6, 1.0);
const PROGRESS_CIRCULAR_DETERMINATE: (f32, f32, f32, f32) = (0.0, 0.0, 0.2, 1.0);
const PROGRESS_LINEAR_INDETERMINATE_CYCLE: Duration = Duration::from_secs(2);
const PROGRESS_CIRCULAR_INDETERMINATE_CYCLE: Duration = Duration::from_millis(5_332);

pub const fn progress() -> ProgressMotion {
    ProgressMotion {
        linear_determinate: Animation::bezier(
            duration::MEDIUM_1,
            PROGRESS_LINEAR_DETERMINATE.0,
            PROGRESS_LINEAR_DETERMINATE.1,
            PROGRESS_LINEAR_DETERMINATE.2,
            PROGRESS_LINEAR_DETERMINATE.3,
        ),
        circular_determinate: Animation::bezier(
            duration::LONG_2,
            PROGRESS_CIRCULAR_DETERMINATE.0,
            PROGRESS_CIRCULAR_DETERMINATE.1,
            PROGRESS_CIRCULAR_DETERMINATE.2,
            PROGRESS_CIRCULAR_DETERMINATE.3,
        ),
        linear_indeterminate_cycle: PROGRESS_LINEAR_INDETERMINATE_CYCLE,
        circular_indeterminate_cycle: PROGRESS_CIRCULAR_INDETERMINATE_CYCLE,
    }
}

/// The caret blinks on the platform text-cursor cadence (530ms per half cycle),
/// which is a system convention rather than an M3 motion token.
const CARET_HALF_CYCLE: Duration = Duration::from_millis(530);

pub const fn text_caret() -> TextCaretMotion {
    TextCaretMotion {
        fade_cycle_duration: CARET_HALF_CYCLE.saturating_mul(2),
        frame_interval: CARET_HALF_CYCLE,
        min_opacity: 0.2,
    }
}

/// The shared-axis outgoing view has faded out by 35% of the transition, so the
/// incoming view fades in over the remainder rather than cross-fading through a
/// muddy midpoint (M3 fade-through).
const NAVIGATION_FADE_THROUGH_THRESHOLD: f32 = 0.35;

pub const fn navigation() -> NavigationMotion {
    NavigationMotion {
        transition_duration: duration::LONG_1,
        transition_easing: easing::EMPHASIZED,
        shared_axis_slide_distance: NAVIGATION_SHARED_AXIS_SLIDE_DISTANCE,
        fade_through_threshold: NAVIGATION_FADE_THROUGH_THRESHOLD,
    }
}

/// MDUI navigation-drawer translation motion.
pub fn navigation_drawer(
    opened: Computed<bool>,
    closed_value: f32,
    opened_value: f32,
) -> impl Signal<Output = f32> {
    dialog_property(
        opened,
        closed_value,
        opened_value,
        material_standard(duration::LONG_2),
        material_standard(duration::SHORT_4),
    )
}

/// MDUI modal navigation-drawer scrim motion.
pub fn navigation_drawer_scrim(
    opened: Computed<bool>,
    closed_value: f32,
    opened_value: f32,
) -> impl Signal<Output = f32> {
    dialog_property(
        opened,
        closed_value,
        opened_value,
        Animation::linear(duration::LONG_2),
        Animation::linear(duration::SHORT_4),
    )
}

/// Switch value motion, matching the mdui reference: thumb slide, thumb size,
/// track color, and outline width all transition together over 200ms with the
/// MD3 standard easing (mdui `transition-duration(short4)` + standard curve).
pub const fn toggle_value() -> Animation {
    material_standard(duration::SHORT_4)
}

/// MDUI tooltip scale transition (`short4` with standard easing).
pub const fn tooltip() -> Animation {
    material_standard(duration::SHORT_4)
}

#[derive(Clone)]
struct DialogProperty {
    opened: Computed<bool>,
    closed_value: f32,
    opened_value: f32,
    opening: Animation,
    closing: Animation,
}

impl Signal for DialogProperty {
    type Output = f32;
    type Guard = <Computed<bool> as Signal>::Guard;

    fn get(&self) -> Self::Output {
        if self.opened.get() {
            self.opened_value
        } else {
            self.closed_value
        }
    }

    fn watch(&self, watcher: impl Fn(Context<Self::Output>) + 'static) -> Self::Guard {
        let closed_value = self.closed_value;
        let opened_value = self.opened_value;
        let opening = self.opening.clone();
        let closing = self.closing.clone();
        self.opened.watch(move |context| {
            let opened = *context.value();
            watcher(
                context
                    .map(|opened| if opened { opened_value } else { closed_value })
                    .with(if opened {
                        opening.clone()
                    } else {
                        closing.clone()
                    }),
            );
        })
    }
}

fn dialog_property(
    opened: Computed<bool>,
    closed_value: f32,
    opened_value: f32,
    opening: Animation,
    closing: Animation,
) -> impl Signal<Output = f32> {
    DialogProperty {
        opened,
        closed_value,
        opened_value,
        opening,
        closing,
    }
}

/// MDUI dialog panel transform motion.
pub fn dialog_transform(
    opened: Computed<bool>,
    closed_value: f32,
    opened_value: f32,
) -> impl Signal<Output = f32> {
    dialog_property(
        opened,
        closed_value,
        opened_value,
        motion(easing::EMPHASIZED_DECELERATE, duration::MEDIUM_4),
        motion(easing::EMPHASIZED_ACCELERATE, duration::SHORT_4),
    )
}

/// MDUI dialog opacity motion.
pub fn dialog_opacity(
    opened: Computed<bool>,
    closed_value: f32,
    opened_value: f32,
) -> impl Signal<Output = f32> {
    dialog_property(
        opened,
        closed_value,
        opened_value,
        Animation::linear(duration::MEDIUM_4),
        Animation::linear(duration::SHORT_4),
    )
}

pub const fn radio_selection() -> RadioSelectionMotion {
    RadioSelectionMotion {
        inner_grow: motion(easing::EMPHASIZED_DECELERATE, duration::MEDIUM_2),
        inner_opacity: Animation::linear(duration::SHORT_1),
        outer_color: Animation::linear(duration::SHORT_1),
    }
}

#[cfg(test)]
mod tests {
    use super::tokens::{duration, easing, motion};
    use super::{
        RIPPLE_OPACITY_IN, RIPPLE_OPACITY_OUT, RIPPLE_RADIUS_IN, RIPPLE_STATE_LAYER_FADE,
        dialog_opacity, dialog_transform, interaction, navigation, navigation_drawer,
        navigation_drawer_scrim, radio_selection, toggle_value, tooltip,
    };
    use core::time::Duration;
    use std::cell::RefCell;
    use std::rc::Rc;
    use waterui::animation::Animation;
    use waterui::{Binding, Signal, SignalExt as _};

    /// Collect the (value, animation) pairs a motion signal emits, so a test can
    /// assert which curve each direction of a transition uses.
    fn record_transitions(
        signal: &impl Signal<Output = f32>,
        opened: &Binding<bool>,
    ) -> Vec<(f32, Animation)> {
        let updates = Rc::new(RefCell::new(Vec::new()));
        let captured = Rc::clone(&updates);
        let _guard = signal.watch(move |context| {
            captured.borrow_mut().push((
                *context.value(),
                context
                    .metadata()
                    .try_get::<Animation>()
                    .expect("a motion update must carry animation metadata"),
            ));
        });
        opened.set(true);
        opened.set(false);
        updates.borrow().clone()
    }

    /// The state layer's opacities are the M3 values, and each transition uses
    /// the ripple's own timings rather than drifting onto arbitrary numbers.
    #[test]
    fn state_layer_motion_uses_the_ripple_timings_and_material_opacities() {
        let motion_spec = interaction();

        assert_eq!(motion_spec.hover_opacity, 0.08);
        assert_eq!(motion_spec.focus_opacity, 0.12);
        assert_eq!(motion_spec.pressed_opacity, 0.12);
        assert_eq!(motion_spec.dragged_opacity, 0.16);

        for cross_fade in [
            motion_spec.hover_enter,
            motion_spec.hover_exit,
            motion_spec.focus_enter,
            motion_spec.focus_exit,
        ] {
            assert_eq!(
                cross_fade,
                motion(easing::STANDARD, RIPPLE_STATE_LAYER_FADE)
            );
        }
        assert_eq!(
            motion_spec.press_grow,
            motion(easing::STANDARD, RIPPLE_RADIUS_IN)
        );
        assert_eq!(
            motion_spec.press_fade_in,
            Animation::linear(RIPPLE_OPACITY_IN)
        );
        assert_eq!(
            motion_spec.press_fade_out,
            Animation::linear(RIPPLE_OPACITY_OUT)
        );
    }

    /// A release must not rewind the growth: the fade-out is gated until the
    /// wave has finished expanding, so the gate equals the grow duration.
    #[test]
    fn press_fade_out_is_gated_until_the_wave_finished_growing() {
        let motion_spec = interaction();
        assert_eq!(motion_spec.minimum_press_duration, RIPPLE_RADIUS_IN);
    }

    /// Fading in faster than the wave grows is what makes a tap read as a single
    /// gesture rather than two separate animations.
    #[test]
    fn ripple_opacity_settles_before_the_wave_finishes_growing() {
        let motion_spec = interaction();
        assert!(RIPPLE_OPACITY_IN < RIPPLE_RADIUS_IN);
        assert!(motion_spec.touch_delay < RIPPLE_OPACITY_IN);
    }

    /// M3 gives an entering container the decelerate curve and a leaving one the
    /// accelerate curve, and closing is quicker than opening.
    #[test]
    fn dialog_enters_decelerating_and_leaves_accelerating() {
        let opened = Binding::bool(false);
        let transitions =
            record_transitions(&dialog_transform(opened.computed(), 0.0, 1.0), &opened);

        assert_eq!(
            transitions.as_slice(),
            &[
                (
                    1.0,
                    motion(easing::EMPHASIZED_DECELERATE, duration::MEDIUM_4)
                ),
                (
                    0.0,
                    motion(easing::EMPHASIZED_ACCELERATE, duration::SHORT_4)
                ),
            ]
        );
    }

    /// Opacity is always linear in M3 — an eased fade reads as a flicker.
    #[test]
    fn dialog_and_scrim_opacity_are_linear() {
        let opened = Binding::bool(false);
        let dialog = record_transitions(&dialog_opacity(opened.computed(), 0.0, 1.0), &opened);
        assert_eq!(
            dialog.as_slice(),
            &[
                (1.0, Animation::linear(duration::MEDIUM_4)),
                (0.0, Animation::linear(duration::SHORT_4)),
            ]
        );

        let opened = Binding::bool(false);
        let scrim = record_transitions(
            &navigation_drawer_scrim(opened.computed(), 0.0, 0.4),
            &opened,
        );
        assert_eq!(
            scrim.as_slice(),
            &[
                (0.4, Animation::linear(duration::LONG_2)),
                (0.0, Animation::linear(duration::SHORT_4)),
            ]
        );
    }

    /// The drawer slides in on standard easing and retracts on the same curve,
    /// faster — dismissal should not feel as deliberate as summoning.
    #[test]
    fn navigation_drawer_closes_faster_than_it_opens() {
        let opened = Binding::bool(false);
        let transitions =
            record_transitions(&navigation_drawer(opened.computed(), -360.0, 0.0), &opened);

        assert_eq!(
            transitions.as_slice(),
            &[
                (0.0, motion(easing::STANDARD, duration::LONG_2)),
                (-360.0, motion(easing::STANDARD, duration::SHORT_4)),
            ]
        );
    }

    /// Small in-place controls use standard easing at `short4`.
    #[test]
    fn small_controls_animate_on_the_standard_short_token() {
        let expected = motion(easing::STANDARD, duration::SHORT_4);
        assert_eq!(toggle_value(), expected);
        assert_eq!(tooltip(), expected);
    }

    /// The radio dot grows on the emphasized decelerate curve while its colour
    /// and opacity snap over at `short1`, so the dot appears to arrive rather
    /// than to fade up.
    #[test]
    fn radio_dot_grows_slower_than_its_colour_changes() {
        let motion_spec = radio_selection();

        assert_eq!(
            motion_spec.inner_grow,
            motion(easing::EMPHASIZED_DECELERATE, duration::MEDIUM_2)
        );
        assert_eq!(
            motion_spec.inner_opacity,
            Animation::linear(duration::SHORT_1)
        );
        assert_eq!(
            motion_spec.outer_color,
            Animation::linear(duration::SHORT_1)
        );
    }

    /// Navigation is a full-screen transition, so it uses the emphasized curve
    /// on a long token rather than the standard/short pairing of a control.
    #[test]
    fn navigation_uses_an_emphasized_long_transition() {
        let motion_spec = navigation();

        assert_eq!(motion_spec.transition_duration, duration::LONG_1);
        assert_eq!(motion_spec.transition_easing, easing::EMPHASIZED);
        assert!(motion_spec.transition_duration > duration::SHORT_4);
    }

    /// The caret spends equal time visible and hidden.
    #[test]
    fn text_caret_blinks_on_a_symmetric_cycle() {
        let motion_spec = super::text_caret();
        assert_eq!(
            motion_spec.fade_cycle_duration,
            motion_spec.frame_interval.saturating_mul(2)
        );
        assert_eq!(motion_spec.frame_interval, Duration::from_millis(530));
    }
}
