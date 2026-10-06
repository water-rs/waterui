//! The `layer_transform` support: the helpers `WuiLayerTransform.swift`
//! defined as free functions — the transform-aware content layout and the
//! explicit `CAAnimation` driving `AppKit` layer transforms.
//!
//! `WuiLayerTransform` was never a registered component — no
//! `Metadata<M>` claims it — so there is no leaf to port, only the shared
//! machinery `rotation` and `scale` call: [`watcher_timing`] maps the
//! watcher metadata's `Animation` to a kit [`Timing`], the shared
//! [`crate::animation::with_platform_animation`] runs a change under it,
//! and [`animate_layer_transform`] applies it as an explicit `transform`
//! keypath animation on `AppKit`.

#[cfg(target_os = "macos")]
use waterui::animation::Animation;
#[cfg(target_os = "macos")]
use waterui::reactive::watcher::Metadata as WatchMetadata;

#[cfg(target_os = "macos")]
use cocoa_ui::core_animation::Timing;

/// `parseAnimation`: the watcher metadata's `Animation` mapped to a kit
/// [`Timing`] by the shared [`crate::animation::timing`] mapping. `None`
/// answers `None`: no animation, the change applies directly.
#[cfg(target_os = "macos")]
pub fn watcher_timing(metadata: &WatchMetadata) -> Option<Timing> {
    metadata
        .try_get::<Animation>()
        .map(|animation| crate::animation::timing(&animation))
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
