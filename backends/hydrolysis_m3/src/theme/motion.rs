use core::time::Duration;

use waterui::animation::Animation;
use waterui_backend_core::widget::{
    InteractionMotion, NavigationMotion, ProgressMotion, RadioSelectionMotion, TextCaretMotion,
};

const MATERIAL_STANDARD: (f32, f32, f32, f32) = (0.2, 0.0, 0.0, 1.0);

pub(crate) fn interaction() -> InteractionMotion {
    InteractionMotion {
        hover_opacity: 0.08,
        focus_opacity: 0.12,
        pressed_opacity: 0.12,
        dragged_opacity: 0.16,
        hover_enter: Animation::linear(Duration::from_millis(15)),
        hover_exit: Animation::linear(Duration::from_millis(15)),
        focus_enter: Animation::bezier(
            Duration::from_millis(150),
            MATERIAL_STANDARD.0,
            MATERIAL_STANDARD.1,
            MATERIAL_STANDARD.2,
            MATERIAL_STANDARD.3,
        ),
        focus_exit: Animation::bezier(
            Duration::from_millis(150),
            MATERIAL_STANDARD.0,
            MATERIAL_STANDARD.1,
            MATERIAL_STANDARD.2,
            MATERIAL_STANDARD.3,
        ),
        press_fade_in: Animation::linear(Duration::from_millis(105)),
        press_fade_out: Animation::linear(Duration::from_millis(375)),
        press_grow: Animation::bezier(
            Duration::from_millis(450),
            MATERIAL_STANDARD.0,
            MATERIAL_STANDARD.1,
            MATERIAL_STANDARD.2,
            MATERIAL_STANDARD.3,
        ),
        minimum_press_duration: Duration::from_millis(225),
        touch_delay: Duration::from_millis(150),
    }
}

pub(crate) fn progress() -> ProgressMotion {
    ProgressMotion {
        linear_determinate: Animation::bezier(Duration::from_millis(250), 0.4, 0.0, 0.6, 1.0),
        circular_determinate: Animation::bezier(Duration::from_millis(500), 0.0, 0.0, 0.2, 1.0),
        linear_indeterminate_cycle: Duration::from_millis(2_000),
        circular_indeterminate_cycle: Duration::from_millis(5_332),
    }
}

pub(crate) fn text_caret() -> TextCaretMotion {
    TextCaretMotion {
        fade_cycle_duration: Duration::from_millis(1_060),
        frame_interval: Duration::from_millis(530),
        min_opacity: 0.2,
    }
}

pub(crate) fn navigation() -> NavigationMotion {
    NavigationMotion {
        transition_duration: Duration::from_millis(250),
        pushpop_parallax_factor: 0.35,
    }
}

pub(crate) fn navigation_drawer() -> Animation {
    Animation::bezier(
        Duration::from_millis(250),
        MATERIAL_STANDARD.0,
        MATERIAL_STANDARD.1,
        MATERIAL_STANDARD.2,
        MATERIAL_STANDARD.3,
    )
}

pub(crate) fn toggle_value() -> Animation {
    Animation::spring(300.0, 20.0)
}

pub(crate) fn radio_selection() -> RadioSelectionMotion {
    RadioSelectionMotion {
        inner_grow: Animation::bezier(Duration::from_millis(300), 0.05, 0.7, 0.1, 1.0),
        inner_opacity: Animation::linear(Duration::from_millis(50)),
        outer_color: Animation::linear(Duration::from_millis(50)),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        interaction, navigation, navigation_drawer, progress, radio_selection, text_caret,
        toggle_value,
    };
    use core::time::Duration;
    use waterui::animation::Animation;

    #[test]
    fn material_state_layer_motion_matches_material_web() {
        let motion = interaction();
        assert_eq!(motion.hover_opacity, 0.08);
        assert_eq!(motion.focus_opacity, 0.12);
        assert_eq!(motion.pressed_opacity, 0.12);
        assert_eq!(motion.dragged_opacity, 0.16);
        assert_eq!(
            motion.hover_enter,
            Animation::linear(Duration::from_millis(15))
        );
        assert_eq!(
            motion.focus_enter,
            Animation::bezier(Duration::from_millis(150), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            motion.focus_exit,
            Animation::bezier(Duration::from_millis(150), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(
            motion.press_fade_in,
            Animation::linear(Duration::from_millis(105))
        );
        assert_eq!(
            motion.press_fade_out,
            Animation::linear(Duration::from_millis(375))
        );
        assert_eq!(
            motion.press_grow,
            Animation::bezier(Duration::from_millis(450), 0.2, 0.0, 0.0, 1.0)
        );
        assert_eq!(motion.minimum_press_duration, Duration::from_millis(225));
        assert_eq!(motion.touch_delay, Duration::from_millis(150));
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
        assert_eq!(
            motion.linear_indeterminate_cycle,
            Duration::from_millis(2_000)
        );
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
    fn material_navigation_drawer_motion_matches_material_web_labs() {
        assert_eq!(
            navigation_drawer(),
            Animation::bezier(Duration::from_millis(250), 0.2, 0.0, 0.0, 1.0)
        );
    }

    #[test]
    fn material_toggle_motion_uses_hydrolysis_animation_engine_policy() {
        assert_eq!(toggle_value(), Animation::spring(300.0, 20.0));
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
