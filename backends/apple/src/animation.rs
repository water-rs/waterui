//! The `Animation` → native timing mapping every animated write shares.
//!
//! `ScrollRequest`s, watcher metadata and component-local animations all
//! carry waterui's `Animation` enum; the backend plays it through
//! `cocoa_ui::core_animation`'s primitives, so the one place the two
//! vocabularies meet is [`timing`]. Frame-driven animations — the scroll
//! flights the kit clock ticks — evaluate the curve themselves through
//! [`progress`] instead.

#[cfg(frame_progress)]
use std::rc::Rc;

#[cfg(platform_timing)]
use cocoa_ui::core_animation::Timing;
#[cfg(any(platform_timing, frame_progress))]
use waterui::animation::Animation;
#[cfg(watcher_animation)]
use waterui::animation::ChangeAnimation;
#[cfg(watcher_animation)]
use waterui::reactive::watcher::Metadata;

/// The kit timing `animation` plays under: its shared timing, the one core
/// and the Cherenkov engine evaluate.
///
/// `Default` is the 0.25s ease-in-out bezier core defines it as; `Bezier`
/// carries the curve's own duration and control points; `Spring` hands the
/// shared spring to a spring primitive (`UISpringTimingParameters` on iOS,
/// `CASpringAnimation` on macOS), which takes its stiffness, damping
/// coefficient and settling duration from it.
#[cfg(platform_timing)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "the control points are authored as f32 and widen to f64 exactly, so they narrow back exactly"
)]
pub fn timing(animation: &Animation) -> Timing {
    match animation.clone().validated().timing() {
        waterui::easing::Timing::Curve { curve, duration } => Timing::Bezier {
            duration,
            control_points: [
                curve.x1 as f32,
                curve.y1 as f32,
                curve.x2 as f32,
                curve.y2 as f32,
            ],
        },
        waterui::easing::Timing::Spring(spring) => Timing::Spring(spring),
    }
}

/// `withPlatformAnimation`: runs `body` under the platform animation
/// `metadata` carries — watcher metadata maps to a kit timing through
/// [`timing`]; a change that names no animation or disables it runs `body`
/// directly. Every component that honors watcher animation metadata
/// funnels here.
#[cfg(watcher_animation)]
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
#[cfg(frame_progress)]
pub fn progress(animation: &Animation) -> Rc<dyn Fn(f64) -> f64> {
    let animation = animation.clone().validated();
    Rc::new(move |elapsed| animation.progress(core::time::Duration::from_secs_f64(elapsed)))
}

#[cfg(all(test, platform_timing))]
mod tests {
    use core::time::Duration;

    use super::*;

    /// `Animation::Default` maps to the 0.25s ease-in-out bezier the FFI
    /// spells the system animation as.
    #[test]
    fn default_maps_to_system_bezier() {
        assert_eq!(
            timing(&Animation::Default),
            Timing::Bezier {
                duration: 0.25,
                control_points: [0.42, 0.0, 0.58, 1.0],
            }
        );
    }

    /// `Animation::Bezier` carries its own duration and control points.
    #[test]
    fn bezier_maps_duration_and_control_points() {
        assert_eq!(
            timing(&Animation::Bezier {
                duration: Duration::from_millis(400),
                x1: 0.1,
                y1: 0.2,
                x2: 0.9,
                y2: 0.8,
            }),
            Timing::Bezier {
                duration: 0.4,
                control_points: [0.1, 0.2, 0.9, 0.8],
            }
        );
    }

    /// `Animation::Spring` hands the platform primitive the shared spring,
    /// whose stiffness and damping coefficient are the authored ones.
    #[test]
    fn spring_maps_stiffness_and_damping() {
        let Timing::Spring(spring) = timing(&Animation::Spring {
            stiffness: 120.0,
            damping: 14.0,
        }) else {
            panic!("a spring plays as a spring");
        };
        assert_eq!(spring, waterui::easing::Spring::from_physics(120.0, 14.0));
        assert!((spring.stiffness() - 120.0).abs() < 1e-9);
        assert!((spring.damping_coefficient() - 14.0).abs() < 1e-9);
    }
}
