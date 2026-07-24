use core::time::Duration;

use waterui::animation::Animation;
use waterui::reactive::watcher::Context;
use waterui::{Computed, Signal};
use waterui_backend_core::widget::{
    InteractionMotion, NavigationMotion, ProgressMotion, RadioSelectionMotion, TextCaretMotion,
};

const MATERIAL_STANDARD: (f32, f32, f32, f32) = (0.2, 0.0, 0.0, 1.0);
const MATERIAL_EMPHASIZED_ACCELERATE: (f32, f32, f32, f32) = (0.3, 0.0, 0.8, 0.15);
const MATERIAL_EMPHASIZED_DECELERATE: (f32, f32, f32, f32) = (0.05, 0.7, 0.1, 1.0);

/// The MD3 standard easing curve over `duration`.
const fn material_standard(duration: Duration) -> Animation {
    Animation::bezier(
        duration,
        MATERIAL_STANDARD.0,
        MATERIAL_STANDARD.1,
        MATERIAL_STANDARD.2,
        MATERIAL_STANDARD.3,
    )
}

/// MD3 interaction state-layer motion, matching the mdui reference
/// implementation: the ripple grows from the press point over 225ms with
/// standard easing while fading in linearly over 75ms; a release never plays
/// the growth backwards — the wave holds its expanded shape and only fades out
/// linearly over 150ms once the expansion has completed
/// (`minimum_press_duration` equals the grow duration for exactly that
/// gating). Hover/focus state layers cross-fade over 280ms with standard
/// easing.
pub const fn interaction() -> InteractionMotion {
    InteractionMotion {
        hover_opacity: 0.08,
        focus_opacity: 0.12,
        pressed_opacity: 0.12,
        dragged_opacity: 0.16,
        hover_enter: material_standard(Duration::from_millis(280)),
        hover_exit: material_standard(Duration::from_millis(280)),
        focus_enter: material_standard(Duration::from_millis(280)),
        focus_exit: material_standard(Duration::from_millis(280)),
        press_fade_in: Animation::linear(Duration::from_millis(75)),
        press_fade_out: Animation::linear(Duration::from_millis(150)),
        press_grow: material_standard(Duration::from_millis(225)),
        minimum_press_duration: Duration::from_millis(225),
        touch_delay: Duration::from_millis(70),
    }
}

pub const fn progress() -> ProgressMotion {
    ProgressMotion {
        linear_determinate: Animation::bezier(Duration::from_millis(250), 0.4, 0.0, 0.6, 1.0),
        circular_determinate: Animation::bezier(Duration::from_millis(500), 0.0, 0.0, 0.2, 1.0),
        linear_indeterminate_cycle: Duration::from_secs(2),
        circular_indeterminate_cycle: Duration::from_millis(5_332),
    }
}

pub const fn text_caret() -> TextCaretMotion {
    TextCaretMotion {
        fade_cycle_duration: Duration::from_millis(1_060),
        frame_interval: Duration::from_millis(530),
        min_opacity: 0.2,
    }
}

pub const fn navigation() -> NavigationMotion {
    NavigationMotion {
        transition_duration: Duration::from_millis(250),
        pushpop_parallax_factor: 0.35,
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
        material_standard(Duration::from_millis(500)),
        material_standard(Duration::from_millis(200)),
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
        Animation::linear(Duration::from_millis(500)),
        Animation::linear(Duration::from_millis(200)),
    )
}

/// Switch value motion, matching the mdui reference: thumb slide, thumb size,
/// track color, and outline width all transition together over 200ms with the
/// MD3 standard easing (mdui `transition-duration(short4)` + standard curve).
pub const fn toggle_value() -> Animation {
    material_standard(Duration::from_millis(200))
}

/// MDUI tooltip scale transition (`short4` with standard easing).
pub const fn tooltip() -> Animation {
    material_standard(Duration::from_millis(200))
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
        Animation::bezier(
            Duration::from_millis(400),
            MATERIAL_EMPHASIZED_DECELERATE.0,
            MATERIAL_EMPHASIZED_DECELERATE.1,
            MATERIAL_EMPHASIZED_DECELERATE.2,
            MATERIAL_EMPHASIZED_DECELERATE.3,
        ),
        Animation::bezier(
            Duration::from_millis(200),
            MATERIAL_EMPHASIZED_ACCELERATE.0,
            MATERIAL_EMPHASIZED_ACCELERATE.1,
            MATERIAL_EMPHASIZED_ACCELERATE.2,
            MATERIAL_EMPHASIZED_ACCELERATE.3,
        ),
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
        Animation::linear(Duration::from_millis(400)),
        Animation::linear(Duration::from_millis(200)),
    )
}

pub const fn radio_selection() -> RadioSelectionMotion {
    RadioSelectionMotion {
        inner_grow: Animation::bezier(Duration::from_millis(300), 0.05, 0.7, 0.1, 1.0),
        inner_opacity: Animation::linear(Duration::from_millis(50)),
        outer_color: Animation::linear(Duration::from_millis(50)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        dialog_opacity, dialog_transform, interaction, navigation, navigation_drawer,
        navigation_drawer_scrim, progress, radio_selection, text_caret, toggle_value, tooltip,
    };
    use core::time::Duration;
    use std::cell::RefCell;
    use std::rc::Rc;
    use waterui::animation::Animation;
    use waterui::{Binding, Signal, SignalExt as _};

    #[test]
    fn material_state_layer_motion_matches_mdui_reference() {
        let motion = interaction();
        assert_eq!(motion.hover_opacity, 0.08);
        assert_eq!(motion.focus_opacity, 0.12);
        assert_eq!(motion.pressed_opacity, 0.12);
        assert_eq!(motion.dragged_opacity, 0.16);
        assert_eq!(
            motion.hover_enter,
            Animation::bezier(Duration::from_millis(280), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            motion.hover_exit,
            Animation::bezier(Duration::from_millis(280), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            motion.focus_enter,
            Animation::bezier(Duration::from_millis(280), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            motion.focus_exit,
            Animation::bezier(Duration::from_millis(280), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            motion.press_fade_in,
            Animation::linear(Duration::from_millis(75))
        );
        assert_eq!(
            motion.press_fade_out,
            Animation::linear(Duration::from_millis(150))
        );
        assert_eq!(
            motion.press_grow,
            Animation::bezier(Duration::from_millis(225), 0.2, 0.0, 0.0, 1.0)
        );
        // The deferred fade-out gate equals the grow duration: a quick tap's
        // ripple finishes expanding before it starts to fade (mdui waits for
        // the radius-in animation to end before applying the fade-out).
        assert_eq!(motion.minimum_press_duration, Duration::from_millis(225));
        assert_eq!(motion.touch_delay, Duration::from_millis(70));
    }

    #[test]
    fn material_progress_motion_matches_material_web() {
        let motion = progress();
        assert_eq!(
            motion.linear_determinate,
            Animation::bezier(Duration::from_millis(250), 0.4, 0.0, 0.6, 1.0)
        );
        assert_eq!(
            motion.circular_determinate,
            Animation::bezier(Duration::from_millis(500), 0.0, 0.0, 0.2, 1.0)
        );
        assert_eq!(motion.linear_indeterminate_cycle, Duration::from_secs(2));
        assert_eq!(
            motion.circular_indeterminate_cycle,
            Duration::from_millis(5_332)
        );
    }

    #[test]
    fn material_text_caret_motion_is_theme_owned() {
        let motion = text_caret();

        assert_eq!(motion.fade_cycle_duration, Duration::from_millis(1_060));
        assert_eq!(motion.frame_interval, Duration::from_millis(530));
        assert_eq!(motion.min_opacity, 0.2);
    }

    #[test]
    fn material_navigation_motion_uses_hydrolysis_transition_engine_policy() {
        let motion = navigation();

        assert_eq!(motion.transition_duration, Duration::from_millis(250));
        assert_eq!(motion.pushpop_parallax_factor, 0.35);
    }

    #[test]
    fn material_navigation_drawer_motion_matches_mdui_2_1_5() {
        let opened = Binding::bool(false);
        let transform = navigation_drawer(opened.computed(), -360.0, 0.0);
        let scrim = navigation_drawer_scrim(opened.computed(), 0.0, 0.4);
        let transform_updates = Rc::new(RefCell::new(Vec::new()));
        let scrim_updates = Rc::new(RefCell::new(Vec::new()));
        let captured_transform_updates = Rc::clone(&transform_updates);
        let captured_scrim_updates = Rc::clone(&scrim_updates);
        let _transform_guard = transform.watch(move |context| {
            captured_transform_updates.borrow_mut().push((
                *context.value(),
                context
                    .metadata()
                    .try_get::<Animation>()
                    .expect("drawer transform update must carry animation metadata"),
            ));
        });
        let _scrim_guard = scrim.watch(move |context| {
            captured_scrim_updates.borrow_mut().push((
                *context.value(),
                context
                    .metadata()
                    .try_get::<Animation>()
                    .expect("drawer scrim update must carry animation metadata"),
            ));
        });

        opened.set(true);
        opened.set(false);

        assert_eq!(
            transform_updates.borrow().as_slice(),
            &[
                (
                    0.0,
                    Animation::bezier(Duration::from_millis(500), 0.2, 0.0, 0.0, 1.0)
                ),
                (
                    -360.0,
                    Animation::bezier(Duration::from_millis(200), 0.2, 0.0, 0.0, 1.0)
                ),
            ]
        );
        assert_eq!(
            scrim_updates.borrow().as_slice(),
            &[
                (0.4, Animation::linear(Duration::from_millis(500))),
                (0.0, Animation::linear(Duration::from_millis(200))),
            ]
        );
    }

    #[test]
    fn material_toggle_motion_matches_mdui_reference() {
        assert_eq!(
            toggle_value(),
            Animation::bezier(Duration::from_millis(200), 0.2, 0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn material_tooltip_motion_matches_mdui_reference() {
        assert_eq!(
            tooltip(),
            Animation::bezier(Duration::from_millis(200), 0.2, 0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn material_dialog_motion_selects_mdui_directional_curves() {
        let opened = Binding::bool(false);
        let transform = dialog_transform(opened.computed(), 0.0, 1.0);
        let opacity = dialog_opacity(opened.computed(), 0.0, 1.0);
        let transform_updates = Rc::new(RefCell::new(Vec::new()));
        let opacity_updates = Rc::new(RefCell::new(Vec::new()));
        let captured_transform_updates = Rc::clone(&transform_updates);
        let captured_opacity_updates = Rc::clone(&opacity_updates);
        let _transform_guard = transform.watch(move |context| {
            captured_transform_updates.borrow_mut().push((
                *context.value(),
                context
                    .metadata()
                    .try_get::<Animation>()
                    .expect("dialog transform update must carry animation metadata"),
            ));
        });
        let _opacity_guard = opacity.watch(move |context| {
            captured_opacity_updates.borrow_mut().push((
                *context.value(),
                context
                    .metadata()
                    .try_get::<Animation>()
                    .expect("dialog opacity update must carry animation metadata"),
            ));
        });

        opened.set(true);
        opened.set(false);

        assert_eq!(
            transform_updates.borrow().as_slice(),
            &[
                (
                    1.0,
                    Animation::bezier(Duration::from_millis(400), 0.05, 0.7, 0.1, 1.0)
                ),
                (
                    0.0,
                    Animation::bezier(Duration::from_millis(200), 0.3, 0.0, 0.8, 0.15)
                ),
            ]
        );
        assert_eq!(
            opacity_updates.borrow().as_slice(),
            &[
                (1.0, Animation::linear(Duration::from_millis(400))),
                (0.0, Animation::linear(Duration::from_millis(200))),
            ]
        );
    }

    #[test]
    fn material_radio_selection_motion_matches_material_web() {
        let motion = radio_selection();

        assert_eq!(
            motion.inner_grow,
            Animation::bezier(Duration::from_millis(300), 0.05, 0.7, 0.1, 1.0)
        );
        assert_eq!(
            motion.inner_opacity,
            Animation::linear(Duration::from_millis(50))
        );
        assert_eq!(
            motion.outer_color,
            Animation::linear(Duration::from_millis(50))
        );
    }
}
