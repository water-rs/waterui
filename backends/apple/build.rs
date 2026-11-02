//! Build-time cfg aliases for the feature unions `crate::animation`
//! gates on.

use cfg_aliases::cfg_aliases;
#[cfg(all(feature = "swiftui", any(target_os = "ios", target_os = "macos")))]
use waterkit_build::SwiftModule;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");

    // The `SwiftUI` embedding bridge compiles header-free under the
    // `swiftui` feature only; every other configuration leaves `swiftc`
    // out of the build.
    #[cfg(feature = "swiftui")]
    compile_waterui_bridge();

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
        // Mac Catalyst: the UIKit realization compiled for macOS.
        catalyst: { all(target_os = "ios", target_abi = "macabi") },
        // The in-process preview entry: the AppKit realization on macOS,
        // the UIKit realization through Mac Catalyst.
        preview_entry: { all(feature = "preview", any(target_os = "macos", catalyst)) },
    }
}

/// Compiles `Sources/WaterUI/Embedding.swift` into `WaterUIBridge` —
/// header-free, with `Extern` for the `@_extern` ABI calls and library
/// evolution for a consumable `.swiftinterface`.
#[cfg(all(feature = "swiftui", any(target_os = "ios", target_os = "macos")))]
fn compile_waterui_bridge() {
    let out_dir = std::path::PathBuf::from(
        std::env::var("OUT_DIR").expect("Cargo sets OUT_DIR for build scripts"),
    );
    let target_os = std::env::var("CARGO_CFG_TARGET_OS")
        .expect("Cargo sets CARGO_CFG_TARGET_OS for build scripts");
    // The backend is aarch64-only (repo AGENTS): `arm64` is `swiftc`'s
    // spelling for it; anything else fails loudly rather than silently
    // targeting a platform the crate does not support.
    let target_arch =
        std::env::var("CARGO_CFG_TARGET_ARCH").expect("Cargo sets CARGO_CFG_TARGET_ARCH");
    let arch = match target_arch.as_str() {
        "aarch64" => "arm64",
        other => panic!("WaterUIBridge requires an aarch64 Apple target, found {other}"),
    };
    let abi = std::env::var("CARGO_CFG_TARGET_ABI").unwrap_or_default();
    // The embedding ABI needs macOS/iOS 26: `Font.resolve(in:)` and its
    // `fontResolutionContext` ship in the 26 SDKs. The deployment floor is
    // the process environment's — the CLI and direct consumers set it.
    let (env_var, suffix) = if target_os == "macos" {
        ("MACOSX_DEPLOYMENT_TARGET", String::new())
    } else {
        assert_eq!(
            target_os, "ios",
            "WaterUIBridge targets Apple platforms only"
        );
        (
            "IPHONEOS_DEPLOYMENT_TARGET",
            match abi.as_str() {
                "" => String::new(),
                "sim" => "-simulator".to_string(),
                "macabi" => "-macabi".to_string(),
                other => panic!("unsupported iOS ABI {other:?} for WaterUIBridge"),
            },
        )
    };
    let deployment = std::env::var(env_var).unwrap_or_else(|_| {
        panic!("waterui-apple's swiftui feature requires {env_var}=26.0 or newer")
    });
    let artifacts = SwiftModule::new("WaterUIBridge")
        .swift_source("Sources/WaterUI/Embedding.swift")
        .framework("Foundation")
        .framework("SwiftUI")
        .framework("CoreText")
        .framework(if target_os == "ios" {
            "UIKit"
        } else {
            "AppKit"
        })
        .compiler_arg("-enable-experimental-feature")
        .compiler_arg("Extern")
        .compiler_arg("-enable-library-evolution")
        // swiftc 6.2.4 answers the plain emission with "emitting module
        // interface files requires '-language-mode'" — the mode is
        // explicit rather than left to a toolchain default.
        .compiler_arg("-language-mode")
        .compiler_arg("5")
        .compiler_arg("-emit-module-interface-path")
        .compiler_arg(
            out_dir
                .join("WaterUIBridge.swiftinterface")
                .into_os_string(),
        )
        .compiler_arg("-target")
        .compiler_arg(format!("{arch}-apple-{target_os}{deployment}{suffix}"))
        .compile();
    println!(
        "cargo:rustc-env=WATERUI_APPLE_SWIFT_MODULE={}",
        artifacts.module().display()
    );
}

/// `SwiftModule` exists on every host; only Apple targets reach its
/// compile — an iOS target built from a non-Apple host has no `swiftc`, and
/// without the feature there is no caller (the env gate in `main` does the
/// dispatch).
#[cfg(all(feature = "swiftui", not(any(target_os = "ios", target_os = "macos"))))]
fn compile_waterui_bridge() {
    panic!("waterui-apple's swiftui feature requires a macOS host with Xcode");
}
