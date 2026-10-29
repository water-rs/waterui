//! Turns the Apple deployment target into one `waterui_apple_os_met` cfg per
//! gated version the target reaches.

use std::process::Command;

include!("versions.rs");

macro_rules! table {
    ($(($major:literal, $minor:literal, $cfg:literal)),* $(,)?) => {
        const VERSIONS: &[(u16, u16, &str)] = &[$(($major, $minor, $cfg)),*];
    };
}
gated_versions!(table);

const DEPLOYMENT_TARGET_VARS: &[&str] = &[
    "MACOSX_DEPLOYMENT_TARGET",
    "IPHONEOS_DEPLOYMENT_TARGET",
    "TVOS_DEPLOYMENT_TARGET",
    "WATCHOS_DEPLOYMENT_TARGET",
    "XROS_DEPLOYMENT_TARGET",
];

fn main() {
    println!("cargo::rerun-if-changed=versions.rs");
    println!("cargo::rerun-if-changed=build.rs");
    for var in DEPLOYMENT_TARGET_VARS {
        println!("cargo::rerun-if-env-changed={var}");
    }
    let cfg_values = VERSIONS
        .iter()
        .map(|(_, _, cfg)| format!("\"{cfg}\""))
        .collect::<Vec<_>>()
        .join(", ");
    println!("cargo::rustc-check-cfg=cfg(waterui_apple_os_met, values({cfg_values}))");
    for (major, minor, cfg) in VERSIONS {
        assert_eq!(
            *cfg,
            format!("{major}.{minor}"),
            "versions.rs row ({major}, {minor}) names the wrong cfg value"
        );
    }

    let vendor =
        std::env::var("CARGO_CFG_TARGET_VENDOR").expect("Cargo sets CARGO_CFG_TARGET_VENDOR");
    // A target that is not Apple has no deployment target to be below.
    let target = (vendor == "apple").then(deployment_target);
    for (major, minor, cfg) in VERSIONS {
        if target.is_none_or(|target| target >= (*major, *minor)) {
            println!("cargo::rustc-cfg=waterui_apple_os_met=\"{cfg}\"");
        }
    }
}

/// The `(major, minor)` deployment target `rustc` resolves for `$TARGET`,
/// which honours the `*_DEPLOYMENT_TARGET` environment.
fn deployment_target() -> (u16, u16) {
    let rustc = std::env::var("RUSTC").expect("Cargo sets RUSTC for build scripts");
    let target = std::env::var("TARGET").expect("Cargo sets TARGET for build scripts");
    let output = Command::new(&rustc)
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
             --target {target}`, which printed {stdout:?}; expected `<VAR>=<major>.<minor>`"
        )
    })
}

/// Reads `IPHONEOS_DEPLOYMENT_TARGET=26.0` (a patch component is ignored).
fn parse_deployment_target(output: &str) -> Option<(u16, u16)> {
    let (_, version) = output.trim().split_once('=')?;
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |minor| minor.parse().ok())?;
    Some((major, minor))
}
