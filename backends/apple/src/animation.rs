//! The `Animation` → native timing mapping every animated write shares.
//!
//! `ScrollRequest`s, watcher metadata and component-local animations all
//! carry waterui's `Animation` enum; the backend plays it through
//! `cocoa_ui::core_animation`'s primitives, so the one place the two
//! vocabularies meet is [`timing`]. Frame-driven animations — the scroll
//! flights the kit clock ticks — evaluate the curve themselves through
//! [`progress`] instead.

use std::rc::Rc;

use cocoa_ui::core_animation::{Playback, Timing};
use nami::watcher::Metadata;
use waterui_core::animation::{Animation, ChangeAnimation};

/// The kit playback specification `animation` plays under: its shared timing,
/// delay, speed, and repeat policy, matching core and the Cherenkov engine.
pub fn timing(animation: &Animation) -> Playback {
    animation.clone().validated().playback()
}

/// The single-cycle duration `metadata` calls for in a cross-dissolve, in seconds.
pub fn cross_dissolve_duration(metadata: &Metadata) -> Option<f64> {
    let animation = ChangeAnimation::of(metadata).animation()?;
    let playback = timing(&animation);
    let speed = playback.effective_speed();
    Some(match playback.timing {
        Timing::Curve { duration, .. } => duration.max(0.0) / speed,
        Timing::Spring(_) => 0.15 / speed,
    })
}

/// `withPlatformAnimation`: runs `body` under the platform animation
/// `metadata` carries — watcher metadata maps to a kit timing through
/// [`timing`]; a change that names no animation or disables it runs `body`
/// directly. Every component that honors watcher animation metadata
/// funnels here.
pub fn with_platform_animation(metadata: &Metadata, body: impl FnOnce() + 'static) {
    let Some(animation) = ChangeAnimation::of(metadata).animation() else {
        return body();
    };
    cocoa_ui::core_animation::animate_with(timing(&animation), body);
}

/// The eased fraction `animation` has reached `elapsed` seconds in — the
/// curve a frame-driven animation evaluates each tick, as the kit writes
/// the model directly rather than handing the property to a platform
/// animator.
pub fn progress(animation: &Animation) -> Rc<dyn Fn(f64) -> f64> {
    let animation = animation.clone().validated();
    Rc::new(move |elapsed| animation.progress(core::time::Duration::from_secs_f64(elapsed)))
}

#[cfg(test)]
mod tests {
    use core::time::Duration;

    use super::*;
    use waterui_core::animation::Repeat;

    /// `Animation::DEFAULT` maps to the 0.25s ease-in-out bezier the FFI
    /// spells the system animation as.
    #[test]
    fn default_maps_to_system_bezier() {
        let playback = timing(&Animation::DEFAULT);
        assert_eq!(playback.delay, 0.0);
        assert_eq!(playback.speed, 1.0);
        assert_eq!(playback.repeat, Repeat::Once);
        let Timing::Curve { curve, duration } = playback.timing else {
            panic!("default animation plays as a bezier curve");
        };
        assert!((duration - 0.25).abs() < 1e-12);
        assert!((curve.x1 - f64::from(0.42_f32)).abs() < 1e-12);
        assert_eq!(curve.y1, 0.0);
        assert!((curve.x2 - f64::from(0.58_f32)).abs() < 1e-12);
        assert_eq!(curve.y2, 1.0);
    }

    /// `Animation::bezier` carries its own duration and control points.
    #[test]
    fn bezier_maps_duration_and_control_points() {
        let playback = timing(&Animation::bezier(
            Duration::from_millis(400),
            0.1,
            0.2,
            0.9,
            0.8,
        ));
        let Timing::Curve { curve, duration } = playback.timing else {
            panic!("bezier animation plays as a bezier curve");
        };
        assert!((duration - 0.4).abs() < 1e-12);
        assert!((curve.x1 - f64::from(0.1_f32)).abs() < 1e-12);
        assert!((curve.y1 - f64::from(0.2_f32)).abs() < 1e-12);
        assert!((curve.x2 - f64::from(0.9_f32)).abs() < 1e-12);
        assert!((curve.y2 - f64::from(0.8_f32)).abs() < 1e-12);
    }

    /// `Animation::spring` hands the platform primitive the shared spring,
    /// whose stiffness and damping coefficient are the authored ones.
    #[test]
    fn spring_maps_stiffness_and_damping() {
        let Timing::Spring(spring) = timing(&Animation::spring(120.0, 14.0)).timing else {
            panic!("a spring plays as a spring");
        };
        assert_eq!(
            spring,
            waterui_core::easing::Spring::from_physics(120.0, 14.0)
        );
        assert!((spring.stiffness() - 120.0).abs() < 1e-9);
        assert!((spring.damping_coefficient() - 14.0).abs() < 1e-9);
    }

    /// `delay`, `speed`, and `repeat` modifiers propagate onto [`Playback`].
    #[test]
    fn modifiers_map_onto_playback() {
        let playback = timing(
            &Animation::linear(Duration::from_millis(500))
                .delay(Duration::from_millis(125))
                .speed(2.0)
                .repeat(Repeat::Count {
                    count: 3,
                    autoreverse: true,
                }),
        );
        assert!((playback.delay - 0.125).abs() < 1e-12);
        assert!((playback.speed - 2.0).abs() < 1e-12);
        assert_eq!(
            playback.repeat,
            Repeat::Count {
                count: 3,
                autoreverse: true,
            }
        );
    }
}
