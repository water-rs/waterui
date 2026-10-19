//! Apple Metal shader toolchain support for build scripts.
//!
//! One copy of the platform plumbing every in-tree build script that
//! precompiles `.metal` sources shares: the cargo Apple target's `xcrun`
//! SDK and `metal -target` triple selection, the deployment-floor check,
//! and the `metal -c` → `metallib` driver. `cherenkov-gpu`'s `build.rs`
//! includes it as `mod metal` and `cocoa-ui`'s `build.rs` reaches it by
//! path, so the rounding flag, the dialect spelling and the triple
//! construction can never drift between the engine shaders and the
//! capture compositor.

#[path = "deployment.rs"]
pub mod deployment;

use std::env;
use std::path::Path;
use std::process::Command;

/// Apple shader tools and the deployment target resolved by rustc.
pub struct AppleTarget {
    /// The SDK `xcrun -sdk` selects for the Metal tools.
    pub sdk: &'static str,
    /// The `metal -target` the air compiles for, spelled like
    /// `air64-apple-ios26.0-macabi`. Every Apple platform passes it
    /// explicitly: the flag alone picks the Metal platform, which ambient
    /// deployment variables cannot express at all for Catalyst and would
    /// otherwise steer (a `MACOSX_DEPLOYMENT_TARGET` in the environment
    /// compiles an iOS-family air for macOS — the misdirection waterui#2158
    /// hit). Once `-target` names the triple, `metal` ignores the
    /// `*_DEPLOYMENT_TARGET` variables entirely and `metallib` never reads
    /// them, so nothing in the `xcrun` environment is scrubbed or set.
    pub metal_target: String,
    /// Whether the air target belongs to the iOS platform family — the
    /// Metal language dialect is ios-keyed for iOS, its simulator and
    /// Catalyst, and macos-keyed only for macOS itself. It falls out of
    /// `metal_target`'s platform segment.
    pub ios_family: bool,
}

/// The Metal tool configuration for an Apple target, or `None` for other
/// targets. A non-Apple host cannot produce a `.metallib`, so building for
/// Apple there is an explicit error — never a silent source fallback.
pub fn apple_target() -> Option<AppleTarget> {
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    // `air_platform`/`air_environment` spell the Rust target's Metal
    // `-target` triple; the iOS family flag is just the platform segment.
    let (sdk, deployment_variable, air_platform, air_environment) = match target_os.as_str() {
        "macos" => ("macosx", "MACOSX_DEPLOYMENT_TARGET", "macosx", ""),
        "ios" if target_env == "sim" => (
            "iphonesimulator",
            "IPHONEOS_DEPLOYMENT_TARGET",
            "ios",
            "-simulator",
        ),
        // Catalyst links the macOS SDK's iOSSupport frameworks, so `metal`
        // runs on the macosx SDK — but the air target and language dialect
        // stay ios-keyed.
        "ios" if target_env == "macabi" => {
            ("macosx", "IPHONEOS_DEPLOYMENT_TARGET", "ios", "-macabi")
        }
        "ios" => ("iphoneos", "IPHONEOS_DEPLOYMENT_TARGET", "ios", ""),
        "tvos" | "watchos" | "visionos" => panic!(
            "Metal shader precompilation for Apple targets: no Metal SDK \
             mapping exists for target-os {target_os}"
        ),
        _ => return None,
    };
    let target = env::var("TARGET").unwrap();
    let host = env::var("HOST").unwrap();
    assert!(
        host.ends_with("apple-darwin"),
        "Metal shaders are precompiled with xcrun: building for {target} \
         needs an Apple host, but the host is {host}. Build for Apple \
         targets on macOS."
    );
    println!("cargo::rerun-if-env-changed={deployment_variable}");
    let output = Command::new(env::var_os("RUSTC").expect("Cargo provides RUSTC"))
        .args(["--print", "deployment-target", "--target", &target])
        .output()
        .expect("rustc must report the Apple deployment target");
    assert!(
        output.status.success(),
        "rustc could not resolve the deployment target for {target}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let resolved = String::from_utf8(output.stdout).expect("rustc deployment target is UTF-8");
    let deployment_version = resolved
        .trim()
        .strip_prefix(&format!("{deployment_variable}="))
        .expect("rustc reports the target platform's deployment variable")
        .to_owned();
    deployment::require_floor(&deployment_version);
    let metal_target = format!("air64-apple-{air_platform}{deployment_version}{air_environment}");
    Some(AppleTarget {
        sdk,
        metal_target,
        ios_family: air_platform == "ios",
    })
}

/// Compiles one `.metal` into `<name>.metallib`. `preinclude` names a
/// generated source the compiler merges ahead of `input` with
/// `-include` — the tile scaffold takes the family's `fs_*` bodies this
/// way instead of naming one module in its own `#include`. Each of
/// `defines` is passed as a preprocessor `-D` definition.
pub fn compile_metal(
    out_dir: &Path,
    name: &str,
    input: &Path,
    apple: &AppleTarget,
    version: (u8, u8),
    preinclude: Option<&Path>,
    defines: &[String],
) {
    let sdk = apple.sdk;
    let air = out_dir.join(format!("{name}.air"));
    let metallib = out_dir.join(format!("{name}.metallib"));
    // Metal 3 unified the platform-specific language dialects.
    let dialect = if version >= (3, 0) {
        "metal"
    } else if apple.ios_family {
        "ios-metal"
    } else {
        "macos-metal"
    };
    let mut metal = Command::new("xcrun");
    metal
        .args(["-sdk", sdk, "metal", "-c", "-o"])
        .arg(&air)
        // Match the language version the module was written/emitted at
        // instead of inheriting the build SDK's newest version, which
        // older supported operating systems cannot load.
        .arg(format!("-std={dialect}{}.{}", version.0, version.1))
        // Apple GPUs store float pixels toward zero by default, which
        // biases every Rgba16Float store one step low and compounds
        // across a pixel's stores; round to nearest even, as the CPU
        // backend and other GPUs do (#2773, #2827). Metal 2.3 and later.
        .arg("-ftexture-write-rounding-mode=rte")
        .arg("-target")
        .arg(&apple.metal_target)
        .arg("-I")
        .arg(out_dir);
    if let Some(preinclude) = preinclude {
        metal.arg("-include").arg(preinclude);
    }
    for define in defines {
        metal.arg(format!("-D{define}"));
    }
    run(
        metal.arg(input),
        name,
        "xcrun metal is required to precompile Metal shaders for Apple \
         targets.",
    );
    run(
        Command::new("xcrun")
            .args(["-sdk", sdk, "metallib", "-o"])
            .arg(&metallib)
            .arg(&air),
        name,
        "xcrun metallib is required to precompile Metal shaders for Apple \
         targets.",
    );
}

/// Runs `command`, failing the build with `hint` when the tool is missing
/// and with the tool's own output when it fails.
pub fn run(command: &mut Command, name: &str, hint: &str) {
    let output = match command.output() {
        Ok(output) => output,
        Err(e) => panic!(
            "{name}: {hint}\nspawning {}: {e}",
            command.get_program().display()
        ),
    };
    assert!(
        output.status.success(),
        "{name}: {} failed ({})\nstdout:\n{}\nstderr:\n{}",
        command.get_program().display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
