//! Build-time cfg aliases for the feature unions `crate::animation`
//! gates on.

use cfg_aliases::cfg_aliases;
#[cfg(all(feature = "swiftui", any(target_os = "ios", target_os = "macos")))]
use waterkit_build::SwiftModule;

include!("src/sf_symbol_releases.rs");

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/sf_symbol_releases.rs");

    facade_features();
    sf_symbol_releases();

    // The `SwiftUI` embedding bridge compiles header-free under the
    // `swiftui` feature only; every other configuration leaves `swiftc`
    // out of the build.
    #[cfg(feature = "swiftui")]
    compile_waterui_bridge();

    // The narrow ObjC shim for headless `aarch64-apple-ios-sim` previews is
    // compiled ONLY when `preview` is enabled on the iOS Simulator target;
    // default/production builds of `waterui-apple` never compile or link it.
    #[cfg(feature = "preview")]
    compile_ios_sim_preview_shim();

    cfg_aliases! {
        // Mac Catalyst: the UIKit realization compiled for macOS.
        catalyst: { all(target_os = "ios", target_abi = "macabi") },
        // Headless iOS Simulator preview entry (`water preview --platform ios`).
        ios_sim_preview: { all(feature = "preview", target_os = "ios", target_abi = "sim") },
        // The in-process preview entry: the AppKit realization on macOS,
        // the headless UIKit realization on iOS Simulator.
        preview_entry: { all(feature = "preview", any(target_os = "macos", ios_sim_preview)) },
        // Opt-in in-process Core Animation render-server capture for windowless
        // previews and native tests on macOS and Mac Catalyst — never active in
        // production builds.
        render_server_capture: { all(any(feature = "preview", feature = "native-test"), any(target_os = "macos", catalyst)) },
    }
}

/// Compiles `src/preview/ios_sim_shim.m` strictly for `aarch64-apple-ios-sim`
/// under `feature = "preview"`. Links only public iOS Simulator SDK frameworks;
/// all runtime hooks resolve via `dlsym` at preview launch.
#[cfg(feature = "preview")]
fn compile_ios_sim_preview_shim() {
    use std::env;

    use cc::Build;

    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    let target_abi = env::var("CARGO_CFG_TARGET_ABI").unwrap_or_default();
    if target_os != "ios" || target_abi != "sim" {
        return;
    }
    println!("cargo::rerun-if-changed=src/preview/ios_sim_shim.m");
    Build::new()
        .file("src/preview/ios_sim_shim.m")
        .flag("-fobjc-arc")
        .compile("waterui_ios_sim_preview_shim");
    println!("cargo::rustc-link-lib=framework=Foundation");
    println!("cargo::rustc-link-lib=framework=UIKit");
    println!("cargo::rustc-link-lib=framework=QuartzCore");
    println!("cargo::rustc-link-lib=framework=CoreGraphics");
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

/// Turns the facade's enabled features into `waterui_feature` cfgs, and
/// fails the build when a facade component is on without the feature that
/// realizes it here: `NativeBackend` then requires a method this crate
/// would not compile.
fn facade_features() {
    // Each facade feature, and the features of this crate that realize its
    // components.
    const REALIZED_BY: &[(&str, &[&str])] = &[
        ("calendar", &["calendar"]),
        ("color-picker", &["color_picker"]),
        ("date-picker", &["date_picker"]),
        ("multi-date-picker", &["multi_date_picker"]),
        ("picker", &["picker"]),
        ("secure-field", &["secure_field"]),
        ("slider", &["slider"]),
        ("stepper", &["stepper"]),
        ("text-field", &["text_field"]),
        ("toggle", &["toggle"]),
        ("menu", &["menu", "context_menu"]),
        ("link", &["link"]),
        ("list", &["list"]),
        ("table", &["table"]),
        ("navigation", &["navigation"]),
        ("dialog", &["dialog"]),
        ("map", &["map"]),
        ("video", &["video", "video_player"]),
        ("canvas", &["canvas"]),
        ("gpu", &["gpu_surface", "picture"]),
    ];
    // Each facade feature satisfied when any of the listed crate features is enabled.
    const REALIZED_BY_ANY: &[(&str, &[&str])] = &[
        ("effects", &["applied_filter", "view_effect"]),
        ("webview", &["webview", "webview-cef"]),
    ];

    println!("cargo::rustc-check-cfg=cfg(waterui_feature, values(any()))");
    let features = std::env::var("DEP_WATERUI_FEATURES")
        .expect("the `waterui` facade's build script exports DEP_WATERUI_FEATURES");
    let enabled = |feature: &str| {
        std::env::var_os(format!(
            "CARGO_FEATURE_{}",
            feature.to_ascii_uppercase().replace('-', "_")
        ))
        .is_some()
    };
    for feature in features.split(',').filter(|feature| !feature.is_empty()) {
        println!("cargo::rustc-cfg=waterui_feature=\"{feature}\"");
        if let Some((_, realized_by)) = REALIZED_BY.iter().find(|(name, _)| *name == feature) {
            let missing: Vec<_> = realized_by.iter().filter(|own| !enabled(own)).collect();
            assert!(
                missing.is_empty(),
                "the application enables the `waterui` feature `{feature}`; enable waterui-apple's {missing:?} to realize it"
            );
        }
        if let Some((_, alternatives)) = REALIZED_BY_ANY.iter().find(|(name, _)| *name == feature) {
            assert!(
                alternatives.iter().any(|own| enabled(own)),
                "the application enables the `waterui` feature `{feature}`; enable one of waterui-apple's {alternatives:?} to realize it"
            );
        }
    }
}

/// Emits `sf_symbol_release` cfgs for every SF Symbols catalog release whose
/// OS version requirement is met by this build's Apple deployment target.
fn sf_symbol_releases() {
    for var in [
        "MACOSX_DEPLOYMENT_TARGET",
        "IPHONEOS_DEPLOYMENT_TARGET",
        "TVOS_DEPLOYMENT_TARGET",
        "WATCHOS_DEPLOYMENT_TARGET",
        "XROS_DEPLOYMENT_TARGET",
    ] {
        println!("cargo::rerun-if-env-changed={var}");
    }

    let values = SF_SYMBOL_RELEASES
        .iter()
        .map(|(key, ..)| format!("\"{key}\""))
        .collect::<Vec<_>>()
        .join(", ");
    println!("cargo::rustc-check-cfg=cfg(sf_symbol_release, values({values}))");

    let vendor =
        std::env::var("CARGO_CFG_TARGET_VENDOR").expect("Cargo sets CARGO_CFG_TARGET_VENDOR");
    if vendor != "apple" {
        return;
    }

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets CARGO_CFG_TARGET_OS");
    let target = apple_deployment_target();
    for &(key, macos, ios, tvos, watchos, visionos) in SF_SYMBOL_RELEASES {
        let required = match target_os.as_str() {
            "macos" => macos,
            "ios" => ios,
            "tvos" => tvos,
            "watchos" => watchos,
            "visionos" => visionos,
            other => panic!("unsupported Apple target_os `{other}` for SfSymbol release gating"),
        };
        if target >= required {
            println!("cargo::rustc-cfg=sf_symbol_release=\"{key}\"");
        }
    }
}

fn apple_deployment_target() -> (u16, u16, u16) {
    let rustc = std::env::var("RUSTC").expect("Cargo sets RUSTC for build scripts");
    let target = std::env::var("TARGET").expect("Cargo sets TARGET for build scripts");
    let output = std::process::Command::new(&rustc)
        .args(["--print", "deployment-target", "--target", &target])
        .output()
        .unwrap_or_else(|error| panic!("cannot run `{rustc} --print deployment-target`: {error}"));
    assert!(
        output.status.success(),
        "`{rustc} --print deployment-target --target {target}` failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_deployment_target(&stdout).unwrap_or_else(|| {
        panic!(
            "cannot parse the deployment target from `{rustc} --print deployment-target \
             --target {target}`, which printed {stdout:?}; expected `<VAR>=<major>.<minor>[.<patch>]`"
        )
    })
}

fn parse_deployment_target(output: &str) -> Option<(u16, u16, u16)> {
    let (_, version) = output.trim().split_once('=')?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |minor| minor.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |patch| patch.parse().ok())?;
    Some((major, minor, patch))
}
