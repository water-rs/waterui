//! Build-time cfg aliases for the feature unions `crate::animation`
//! gates on.

use cfg_aliases::cfg_aliases;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    cfg_aliases! {
        // Rotation and scale animate through `with_platform_animation` on
        // iOS; on macOS they reach `timing` via the layer-transform
        // machinery the two components share.
        ios_transform_animation: {
            all(target_os = "ios", any(feature = "rotation", feature = "scale"))
        },
        // The `WuiLayerTransform` helpers `rotation` and `scale` call —
        // `AppKit` only, since iOS transforms animate through
        // `with_platform_animation` instead.
        macos_layer_transform: {
            all(target_os = "macos", any(feature = "rotation", feature = "scale"))
        },
        // Components that honor watcher animation metadata through
        // `with_platform_animation` (split so each expression stays within
        // the macro's recursion budget).
        container_animation: {
            any(feature = "container", feature = "list", feature = "menu", feature = "table")
        },
        property_animation: { any(feature = "offset", feature = "opacity") },
        watcher_animation: {
            any(container_animation, property_animation, ios_transform_animation)
        },
        // Everything that maps an `Animation` to a kit `Timing`.
        platform_timing: { any(watcher_animation, macos_layer_transform) },
        // The frame-driven scroll flights evaluating the curve per tick.
        frame_progress: { any(feature = "scroll", feature = "list") },
    }
}
