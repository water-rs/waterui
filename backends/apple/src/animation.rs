//! The `Animation` → native timing mapping every animated write shares.
//!
//! `ScrollRequest`s, watcher metadata and component-local animations all
//! carry waterui's `Animation` enum; the backend plays it through
//! `cocoa_ui::core_animation`'s primitives, so the one place the two
//! vocabularies meet is [`timing`]. Frame-driven animations — the scroll
//! flights the kit clock ticks — evaluate the curve themselves through
//! [`progress`] instead.

use std::rc::Rc;

use cocoa_ui::core_animation::Timing;
use waterui::animation::Animation;
use waterui::reactive::watcher::Metadata;

/// The kit timing `animation` plays under.
///
/// `Default` parses to the 0.25s ease-in-out bezier the FFI spells the
/// system animation as; `Bezier` carries the curve's own duration and
/// control points; `Spring` forwards stiffness and damping to a spring
/// primitive (`UISpringTimingParameters` on iOS, `CASpringAnimation` on
/// macOS).
pub fn timing(animation: &Animation) -> Timing {
    match *animation {
        Animation::Default => Timing::Bezier {
            duration: 0.25,
            control_points: [0.42, 0.0, 0.58, 1.0],
        },
        Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        } => Timing::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        },
        Animation::Spring { stiffness, damping } => Timing::Spring {
            stiffness: f64::from(stiffness),
            damping: f64::from(damping),
        },
    }
}

/// `withPlatformAnimation`: runs `body` under the platform animation
/// `metadata` carries — watcher metadata maps to a kit timing through
/// [`timing`]; metadata without an `Animation` runs `body` directly.
/// Every component that honors watcher animation metadata funnels here.
pub fn with_platform_animation(metadata: &Metadata, body: impl FnOnce() + 'static) {
    let Some(animation) = metadata.try_get::<Animation>() else {
        return body();
    };
    cocoa_ui::core_animation::animate_with(timing(&animation), body);
}

/// The eased fraction `animation` has reached `elapsed` seconds in — the
/// curve a frame-driven animation evaluates each tick, as the kit writes
/// the model directly rather than handing the property to a platform
/// animator.
pub fn progress(animation: &Animation) -> Rc<dyn Fn(f64) -> f64> {
    let animation = animation.clone();
    Rc::new(move |elapsed| {
        f64::from(animation.progress(core::time::Duration::from_secs_f64(elapsed)))
    })
}

#[cfg(test)]
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

    /// `Animation::Spring` forwards stiffness and damping to a spring
    /// primitive unchanged.
    #[test]
    fn spring_maps_stiffness_and_damping() {
        assert_eq!(
            timing(&Animation::Spring {
                stiffness: 120.0,
                damping: 14.0,
            }),
            Timing::Spring {
                stiffness: 120.0,
                damping: 14.0,
            }
        );
    }
}
