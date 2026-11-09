//! The `layer_transform` support: the helpers `WuiLayerTransform.swift`
//! defined as free functions — the anchored `sublayerTransform` math and
//! the explicit `CAAnimation` driving `AppKit` layer transforms.
//!
//! `WuiLayerTransform` was never a registered component — no
//! `Metadata<M>` claims it — so there is no leaf to port, only the shared
//! machinery `rotation` and `scale` call: [`watcher_timing`] maps the
//! watcher metadata's `Animation` to a kit [`Timing`], the shared
//! [`crate::animation::with_platform_animation`] runs a change under it,
//! and [`animate_sublayer_transform`] applies it as an explicit
//! `sublayerTransform` keypath animation on `AppKit`.
//!
//! On `AppKit` the transform lives on the wrapper layer's
//! `sublayerTransform`, never the content layer's `transform`: `AppKit`
//! rewrites a layer-backed view's layer geometry at display time, which
//! reset the modifier's transform to identity (#3504), while a parent's
//! `sublayerTransform` is not part of that sync.

use waterui::animation::Animation;
use waterui::reactive::watcher::Metadata as WatchMetadata;

use cocoa_ui::core_animation::Timing;
use cocoa_ui::objc2_quartz_core::CATransform3D;

/// `parseAnimation`: the watcher metadata's `Animation` mapped to a kit
/// [`Timing`] by the shared [`crate::animation::timing`] mapping. `None`
/// answers `None`: no animation, the change applies directly.
pub fn watcher_timing(metadata: &WatchMetadata) -> Option<Timing> {
    metadata
        .try_get::<Animation>()
        .map(|animation| crate::animation::timing(&animation))
}

/// The wrapper's backing layer — the `sublayerTransform` target —
/// `AppKit` only.
///
/// # Panics
///
/// Panics when `wrapper` is not layer-backed — the modifiers call
/// [`cocoa_ui::layer::ensure_layer`] on it at mount.
pub fn wrapper_transform_layer(
    wrapper: &cocoa_ui::PlatformView,
) -> cocoa_ui::Retained<cocoa_ui::objc2_quartz_core::CALayer> {
    cocoa_ui::layer::layer_of(wrapper).expect("a transform wrapper is layer-backed")
}

/// `T(A) · M · T(−A)`: `transform` pivoted at `anchor` — a `0.0…1.0`
/// fraction of the wrapper's `bounds_size` — inside the wrapper's
/// top-left bounds coordinates. `AppKit` only.
///
/// The anchor maps to itself, so the matrix is what the wrapper layer's
/// `sublayerTransform` must hold; `CATransform3DConcat(a, b)` applies `a`
/// first, hence `T(−A)` leads.
#[must_use]
pub fn anchored(
    transform: CATransform3D,
    anchor: cocoa_ui::Point,
    bounds_size: cocoa_ui::Size,
) -> CATransform3D {
    let ax = bounds_size.width * anchor.x;
    let ay = bounds_size.height * anchor.y;
    CATransform3D::new_translation(-ax, -ay, 0.0)
        .concat(transform)
        .concat(CATransform3D::new_translation(ax, ay, 0.0))
}

/// `wuiApplyLayerTransform`: move `layer`'s `sublayerTransform` to
/// `transform` under the watcher metadata's animation — an explicit
/// `CAAnimation` on the `sublayerTransform` keypath, or a
/// transaction-guarded direct set when the metadata carries none. `key`
/// identifies the animation so a later change replaces it — `AppKit`
/// only.
pub fn animate_sublayer_transform(
    layer: &cocoa_ui::objc2_quartz_core::CALayer,
    transform: CATransform3D,
    key: &str,
    metadata: &WatchMetadata,
) {
    cocoa_ui::core_animation::animate_sublayer_transform(
        layer,
        transform,
        key,
        watcher_timing(metadata),
    );
}
