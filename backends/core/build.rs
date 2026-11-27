//! Turns the framework's enabled features into `waterui_feature` cfgs and
//! emits Apple deployment-target `sf_symbol_release` cfgs.
//!
//! `waterui-core` exports them as `DEP_WATERUI_FEATURES` (its `build.rs`), and
//! `NativeBackend` declares a method for a component only when the feature
//! providing it is enabled.

use std::env;

include!("src/sf_symbol_releases.rs");

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-changed=src/sf_symbol_releases.rs");
    println!("cargo::rustc-check-cfg=cfg(waterui_feature, values(any()))");
    let features = env::var("DEP_WATERUI_FEATURES")
        .expect("`waterui-core`'s build script exports DEP_WATERUI_FEATURES");
    for feature in features.split(',').filter(|feature| !feature.is_empty()) {
        println!("cargo::rustc-cfg=waterui_feature=\"{feature}\"");
    }
    sf_symbol_releases();
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

    let vendor = env::var("CARGO_CFG_TARGET_VENDOR").expect("Cargo sets CARGO_CFG_TARGET_VENDOR");
    if vendor != "apple" {
        return;
    }

    let target_os = env::var("CARGO_CFG_TARGET_OS").expect("Cargo sets CARGO_CFG_TARGET_OS");
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
    let rustc = env::var("RUSTC").expect("Cargo sets RUSTC for build scripts");
    let target = env::var("TARGET").expect("Cargo sets TARGET for build scripts");
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
