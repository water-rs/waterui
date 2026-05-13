use core::time::Duration;

use waterui::animation::Animation;
use waterui_backend_core::widget::{InteractionMotion, ProgressMotion};

const MATERIAL_STANDARD: (f32, f32, f32, f32) = (0.2, 0.0, 0.0, 1.0);

pub(crate) fn interaction() -> InteractionMotion {
    InteractionMotion {
        hover_opacity: 0.08,
        focus_opacity: 0.12,
        pressed_opacity: 0.12,
        dragged_opacity: 0.16,
        hover_enter: Animation::linear(Duration::from_millis(15)),
        hover_exit: Animation::linear(Duration::from_millis(15)),
        focus_enter: Animation::linear(Duration::from_millis(15)),
        focus_exit: Animation::linear(Duration::from_millis(15)),
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

#[cfg(test)]
mod tests {
    use super::{interaction, progress};
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
}
