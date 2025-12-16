//! Apple platform build and package utilities.
//!
//! This module provides utility functions for building and packaging Apple apps.
//! These functions are used by `AppleBackend` to implement the `Backend` trait.

use std::path::PathBuf;
use std::{env, fmt::Write};

use color_eyre::eyre::{self, bail};
use smol::fs;
use target_lexicon::Architecture;

use crate::{
    apple::backend::AppleBackend,
    build::{BuildOptions, RustBuild},
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::{copy_file, run_command},
};

// ============================================================================
// Build Utilities
// ============================================================================

/// Build Rust library for an Apple platform.
pub async fn build_rust_lib(
    project: &Project,
    platform: TargetPlatform,
    options: BuildOptions,
) -> eyre::Result<PathBuf> {
    let triple = platform.triple();
    let build = RustBuild::new(project.root(), triple.clone(), options.is_hot_reload());
    let lib_dir = build.build_lib(options.is_release()).await?;

    // If output_dir is specified, copy the library there
    if let Some(output_dir) = options.output_dir() {
        let lib_name = project.crate_name().replace('-', "_");
        let source_lib = lib_dir.join(format!("lib{lib_name}.a"));

        if source_lib.exists() {
            fs::create_dir_all(output_dir).await?;
            let dest_lib = output_dir.join("libwaterui_app.a");
            copy_file(&source_lib, &dest_lib).await?;
        }
    }

    Ok(lib_dir)
}

// ============================================================================
// Validation
// ============================================================================

async fn validate_local_apple_backend(project: &Project) -> eyre::Result<()> {
    let Some(waterui_path) = project.manifest().waterui_path.as_deref() else {
        return Ok(());
    };

    let waterui_root = {
        let candidate = PathBuf::from(waterui_path);
        if candidate.is_absolute() {
            candidate
        } else {
            project.root().join(candidate)
        }
    };

    let package_manifest = waterui_root.join("backends/apple/Package.swift");
    if package_manifest.exists() {
        return Ok(());
    }

    let gitmodules_path = waterui_root.join(".gitmodules");
    let submodule_hint = if gitmodules_path.exists() {
        fs::read_to_string(&gitmodules_path)
            .await
            .ok()
            .filter(|c| c.contains("backends/apple"))
            .map(|_| {
                format!(
                    "It looks like `backends/apple` is a git submodule; run `git submodule update --init --recursive` in `{}`.",
                    waterui_root.display()
                )
            })
    } else {
        None
    };

    let mut message = format!(
        "Local Apple backend Swift package manifest not found at `{}`.\n\
         This is typically caused by an incomplete local WaterUI checkout (e.g. missing submodules) or an incorrect `waterui_path` in `Water.toml`.\n",
        package_manifest.display()
    );

    if let Some(hint) = submodule_hint {
        writeln!(&mut message, "{hint}\n").unwrap();
    } else {
        writeln!(
            &mut message,
            "If you're using a local WaterUI checkout, ensure `backends/apple/` exists and contains `Package.swift`."
        ).unwrap();
    }

    bail!("{message}");
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Xcode build artifacts for an Apple platform.
pub async fn clean_apple(project: &Project) -> eyre::Result<()> {
    let Some(backend) = project.apple_backend() else {
        return Ok(()); // Nothing to clean if no backend configured
    };

    let project_path = project.backend_path::<AppleBackend>();
    let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

    if !xcodeproj.exists() {
        return Ok(());
    }

    run_command(
        "xcodebuild",
        [
            "-project",
            xcodeproj.to_str().unwrap_or_default(),
            "-scheme",
            &backend.scheme,
            "clean",
        ],
    )
    .await?;

    let build_dir = project_path.join("build");
    if build_dir.exists() {
        fs::remove_dir_all(&build_dir).await?;
    }

    Ok(())
}

// ============================================================================
// Package
// ============================================================================

/// Package an Apple app using xcodebuild.
pub async fn package_apple(
    project: &Project,
    platform: TargetPlatform,
    options: PackageOptions,
) -> eyre::Result<Artifact> {
    let backend = project
        .apple_backend()
        .ok_or_else(|| eyre::eyre!("Apple backend must be configured"))?;

    let project_path = project.backend_path::<AppleBackend>();
    let xcodeproj = project_path.join(format!("{}.xcodeproj", backend.scheme));

    if !xcodeproj.exists() {
        bail!(
            "Xcode project not found at {}. Did you run 'water create'?",
            xcodeproj.display()
        );
    }

    validate_local_apple_backend(project).await?;

    // Tell Xcode not to call `water build` again (we already built)
    // SAFETY: CLI runs on main thread before spawning build processes
    unsafe {
        env::set_var("WATERUI_SKIP_RUST_BUILD", "1");
    }

    let configuration = if options.is_debug() {
        "Debug"
    } else {
        "Release"
    };

    let derived_data = project_path.join(".water/DerivedData");
    let target_dir = project.target_dir();
    let triple = platform.triple();

    // Copy the built Rust library to where Xcode expects it
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    let lib_dir = target_dir.join(triple.to_string()).join(profile);
    let lib_name = project.crate_name().replace('-', "_");
    let source_lib = lib_dir.join(format!("lib{lib_name}.a"));

    // Get SDK name - must be an Apple platform
    let sdk_name = platform.sdk_name().ok_or_else(|| {
        eyre::eyre!("Platform {:?} is not an Apple platform", platform)
    })?;

    // Xcode uses "Debug-iphonesimulator" for simulators, "Debug" for macOS
    let products_config = if sdk_name == "macosx" {
        configuration.to_string()
    } else {
        format!("{configuration}-{sdk_name}")
    };
    let products_dir = derived_data.join("Build/Products").join(&products_config);
    fs::create_dir_all(&products_dir).await?;
    let dest_lib = products_dir.join("libwaterui_app.a");
    copy_file(&source_lib, &dest_lib).await?;

    // Build with xcodebuild
    // Determine the Xcode arch name from the platform architecture
    let arch_name = match platform.arch() {
        Architecture::Aarch64(_) => "arm64",
        Architecture::X86_64 => "x86_64",
        _ => unimplemented!(),
    };
    let archs_arg = format!("ARCHS={arch_name}");

    let mut args = vec![
        "-project",
        xcodeproj.to_str().unwrap_or_default(),
        "-scheme",
        &backend.scheme,
        "-configuration",
        configuration,
        "-sdk",
        sdk_name,
        "-derivedDataPath",
        derived_data.to_str().unwrap_or_default(),
        &archs_arg,
        "ONLY_ACTIVE_ARCHITECTURE=YES",
        "build",
    ];

    if platform.is_simulator() || options.is_debug() {
        args.extend([
            "CODE_SIGNING_ALLOWED=NO",
            "CODE_SIGNING_REQUIRED=NO",
            "CODE_SIGN_IDENTITY=-",
        ]);
    }

    run_command("xcodebuild", args.iter().copied()).await?;

    // Reset the environment variable
    unsafe {
        env::set_var("WATERUI_SKIP_RUST_BUILD", "0");
    }

    let app_path = products_dir.join(format!("{}.app", backend.scheme));

    if !app_path.exists() {
        bail!(
            "Built app not found at {}. Check xcodebuild output for errors.",
            app_path.display()
        );
    }

    Ok(Artifact::new(project.bundle_identifier(), app_path))
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the Apple backend.
pub const fn is_apple_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::MacOS
            | TargetPlatform::IOS
            | TargetPlatform::IOSSimulator
            | TargetPlatform::TvOS
            | TargetPlatform::TvOSSimulator
            | TargetPlatform::WatchOS
            | TargetPlatform::WatchOSSimulator
            | TargetPlatform::VisionOS
            | TargetPlatform::VisionOSSimulator
    )
}
