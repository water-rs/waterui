//! The `layer_transform` support: the helpers `WuiLayerTransform.swift`
//! defined as free functions — the transform-aware content layout and the
//! explicit `CAAnimation` driving `AppKit` layer transforms.
//!
//! `WuiLayerTransform` was never a registered component — no
//! `Metadata<M>` claims it — so there is no leaf to port, only the shared
//! machinery `rotation` and `scale` call: [`watcher_timing`] maps the
//! watcher metadata's `Animation` to a kit [`Timing`],
//! [`apply_with_animation`] runs a change under it on `UIKit`, and
//! [`animate_layer_transform`] applies it as an explicit `transform`
//! keypath animation on `AppKit`.

use waterui::animation::Animation;
use waterui::reactive::watcher::Metadata as WatchMetadata;

use cocoa_ui::core_animation::Timing;

/// `parseAnimation`: the watcher metadata's `Animation` mapped to a kit
/// [`Timing`] — `Default` parses to the 0.25s bezier the FFI spells it as.
/// `None` answers `None`: no animation, the change applies directly.
pub fn watcher_timing(metadata: &WatchMetadata) -> Option<Timing> {
    match metadata.try_get::<Animation>() {
        None => None,
        Some(Animation::Default) => Some(Timing::Bezier {
            duration: 0.25,
            control_points: [0.42, 0.0, 0.58, 1.0],
        }),
        Some(Animation::Bezier {
            duration,
            x1,
            y1,
            x2,
            y2,
        }) => Some(Timing::Bezier {
            duration: duration.as_secs_f64(),
            control_points: [x1, y1, x2, y2],
        }),
        Some(Animation::Spring { stiffness, damping }) => Some(Timing::Spring {
            stiffness: f64::from(stiffness),
            damping: f64::from(damping),
        }),
    }
}

/// `withPlatformAnimation`: run `body` under [`watcher_timing`] — an
/// `NSAnimationContext`/`UIViewPropertyAnimator` wrapping the imperative
/// platform write.
#[cfg(target_os = "ios")]
pub fn apply_with_animation(metadata: &WatchMetadata, body: impl FnOnce() + 'static) {
    match watcher_timing(metadata) {
        None => body(),
        Some(timing) => cocoa_ui::core_animation::animate_with(timing, body),
    }
}

/// `transformedContentLayer`: lay the transformed content out for its
/// anchor, then answer its layer — `AppKit` only.
///
/// # Panics
///
/// Panics when `child` is not layer-backed after the transformed layout —
/// the same `fatalError` `WuiRotation`/`WuiScale` declared.
#[cfg(target_os = "macos")]
pub fn transformed_content_layer(
    child: &cocoa_ui::PlatformView,
    container_bounds: cocoa_ui::Rect,
    anchor: cocoa_ui::Point,
    last_bounds_size: &mut cocoa_ui::Size,
) -> cocoa_ui::Retained<cocoa_ui::objc2_quartz_core::CALayer> {
    cocoa_ui::layer::layout_transformed_content(child, container_bounds, anchor, last_bounds_size);
    cocoa_ui::layer::layer_of(child).expect("transformed AppKit content must be layer-backed")
}

/// `wuiApplyLayerTransform`: move `layer`'s transform to `transform` under
/// the watcher metadata's animation — an explicit `CAAnimation` on the
/// `transform` keypath, or a transaction-guarded direct set when the
/// metadata carries none. `key` identifies the animation so a later change
/// replaces it — `AppKit` only.
#[cfg(target_os = "macos")]
pub fn animate_layer_transform(
    layer: &cocoa_ui::objc2_quartz_core::CALayer,
    transform: cocoa_ui::objc2_quartz_core::CATransform3D,
    key: &str,
    metadata: &WatchMetadata,
) {
    cocoa_ui::core_animation::animate_layer_transform(
        layer,
        transform,
        key,
        watcher_timing(metadata),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn missing_animation_means_no_timing() {
        assert_eq!(watcher_timing(&WatchMetadata::new()), None);
    }

    #[test]
    fn default_animation_maps_to_the_system_bezier() {
        let metadata = WatchMetadata::new().with(Animation::Default);
        assert_eq!(
            watcher_timing(&metadata),
            Some(Timing::Bezier {
                duration: 0.25,
                control_points: [0.42, 0.0, 0.58, 1.0],
            })
        );
    }

    #[test]
    fn bezier_animation_carries_duration_and_control_points() {
        let metadata = WatchMetadata::new().with(Animation::Bezier {
            duration: Duration::from_millis(500),
            x1: 0.1,
            y1: 0.2,
            x2: 0.3,
            y2: 0.4,
        });
        assert_eq!(
            watcher_timing(&metadata),
            Some(Timing::Bezier {
                duration: 0.5,
                control_points: [0.1, 0.2, 0.3, 0.4],
            })
        );
    }

    #[test]
    fn spring_animation_carries_stiffness_and_damping() {
        let metadata = WatchMetadata::new().with(Animation::Spring {
            stiffness: 200.0,
            damping: 20.0,
        });
        assert_eq!(
            watcher_timing(&metadata),
            Some(Timing::Spring {
                stiffness: 200.0,
                damping: 20.0,
            })
        );
    }
}
