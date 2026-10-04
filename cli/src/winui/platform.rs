//! `WinUI` platform build and package utilities.
//!
//! This module provides utility functions for building and packaging `WinUI` apps.
//! These functions are used by `WinUiBackend` to implement the `Backend` trait.

use std::ffi::OsString;
use std::path::Path;

use eyre::bail;
use futures_util::StreamExt as _;
use smol::fs;
use tracing::info;

use crate::{
    assets,
    build::{BuildOptions, BuiltTarget, RustBuild, RustDynamicLibraries, RustLinkage},
    device::Artifact,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::run_command_os,
    winui::backend::WinUiBackend,
};

#[cfg(target_os = "windows")]
const WINUI_INIT_HINT: &str = "water run --platform windows --backend winui";
#[cfg(not(target_os = "windows"))]
const WINUI_INIT_HINT: &str = "initialize WinUI backend on Windows";

// ============================================================================
// Build Utilities
// ============================================================================

/// Build `WinUI` binary for the host platform.
///
/// # Errors
/// Returns an error if the backend manifest is missing, the host is unsupported, or Cargo fails.
pub async fn build_winui(project: &Project, options: BuildOptions) -> eyre::Result<BuiltTarget> {
    ensure_windows_host()?;

    let backend_path = project.backend_path::<WinUiBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        bail!(
            "WinUI backend not found at {}. Run `{WINUI_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    // The generated `build.rs` embeds `app-icon.ico` into the executable's
    // resources when targeting Windows, so it has to exist before the
    // backend compiles. Assets and fonts stage after the build instead: the
    // mount metadata they need is read from the library artifact this build
    // produces.
    fs::write(
        backend_path.join("app-icon.ico"),
        assets::project_windows_ico(project)?,
    )
    .await?;

    let mut build = RustBuild::new(&backend_path, TargetPlatform::Windows.triple())
        .with_project(project)
        .with_target_dir(project.water_target_dir(options.linkage()).await?)
        .with_linkage(
            options.linkage(),
            &format!("{}/dev", project.crate_name()),
            &[],
        )
        .with_envs(options.cargo_envs().iter().cloned());
    if let Some(sccache_path) = options.sccache_path() {
        build = build.with_sccache(sccache_path.to_path_buf());
    }
    if let Some(progress) = options.progress() {
        build = build.with_progress(progress.clone());
    }
    let built_target = build
        .build_binary(
            project.winui_backend_crate_name().as_str(),
            options.is_release(),
        )
        .await
        .map_err(|error| eyre::eyre!("Failed to build WinUI backend with cargo: {error}"))?;

    copy_assets_and_fonts(
        project,
        &backend_path,
        &built_target.app_symbols()?,
        options.uses_dev_server(),
    )
    .await?;
    Ok(built_target)
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Cargo build artifacts for `WinUI`.
///
/// # Errors
/// Returns an error if the host is unsupported or `cargo clean` fails.
pub async fn clean_winui(project: &Project) -> eyre::Result<()> {
    ensure_windows_host()?;

    let backend_path = project.backend_path::<WinUiBackend>();
    let cargo_toml = backend_path.join("Cargo.toml");
    if !cargo_toml.exists() {
        return Ok(()); // Nothing to clean
    }

    // The target directories are shared with every other generated backend, so only
    // this backend's own package is cleaned — its dependency artifacts stay for
    // the other backends that resolve them identically.
    for linkage in [RustLinkage::SharedRuntime, RustLinkage::Static] {
        let backend_target_dir = project.water_target_dir(linkage).await?;
        if !backend_target_dir.exists() {
            continue;
        }
        let args: Vec<OsString> = vec![
            "clean".into(),
            "--manifest-path".into(),
            cargo_toml.as_os_str().to_owned(),
            "--target-dir".into(),
            backend_target_dir.as_os_str().to_owned(),
            "--package".into(),
            project.winui_backend_crate_name().as_str().into(),
        ];
        run_command_os("cargo", args).await?;
    }

    Ok(())
}

// ============================================================================
// Package
// ============================================================================

/// Package a `WinUI` app (locate the built binary and stage its resources).
///
/// # Errors
/// Returns an error if the host is unsupported, assets cannot be staged, or the built binary is missing.
pub async fn package_winui(
    project: &Project,
    options: PackageOptions,
    built: &BuiltTarget,
) -> eyre::Result<Artifact> {
    ensure_windows_host()?;

    let backend_path = project.backend_path::<WinUiBackend>();

    // Copy project assets and dependency fonts
    copy_assets_and_fonts(
        project,
        &backend_path,
        &built.app_symbols()?,
        options.uses_dev_server(),
    )
    .await?;

    let final_binary_path = &built.artifact;
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };

    // The shipped binary and everything it resolves beside itself stage
    // into the project's own managed backend directory — the shared Cargo
    // profile directory would collide two same-named projects on
    // `<profile>/<product>`.
    let runtime_dir =
        crate::platforming::packaging::dist_dir(&backend_path, "windows", Some(profile));
    fs::create_dir_all(&runtime_dir).await?;

    // The runtime resolves bundled assets relative to the executable, so the
    // staged `resources/` directory must sit next to the produced binary.
    let staged_resources = backend_path.join("resources");
    if staged_resources.is_dir() {
        copy_dir(&staged_resources, &runtime_dir.join("resources")).await?;
    }

    if options.uses_shared_rust_runtime() {
        RustDynamicLibraries::resolve(built, &TargetPlatform::Windows.triple(), project)
            .await?
            .stage(&runtime_dir)
            .await?;
    } else {
        RustDynamicLibraries::remove_staged(&runtime_dir, &TargetPlatform::Windows.triple())
            .await?;
    }

    // Ship the binary under the product name; the tagged Cargo artifact name
    // is internal to the shared target directory.
    let packaged_binary = crate::platforming::packaging::stage_binary_as(
        final_binary_path,
        &runtime_dir,
        &format!("{}.exe", project.winui_binary_name()),
    )
    .await?;

    Ok(Artifact::new(project.bundle_identifier(), packaged_binary))
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the `WinUI` backend.
#[must_use]
pub const fn is_winui_platform(platform: TargetPlatform) -> bool {
    matches!(platform, TargetPlatform::Windows)
}

fn ensure_windows_host() -> eyre::Result<()> {
    if cfg!(target_os = "windows") {
        Ok(())
    } else {
        bail!("WinUI backend is only supported on Windows hosts");
    }
}

/// Copy a directory tree recursively.
async fn copy_dir(source: &Path, destination: &Path) -> eyre::Result<()> {
    let mut stack = vec![(source.to_path_buf(), destination.to_path_buf())];
    while let Some((source, destination)) = stack.pop() {
        fs::create_dir_all(&destination).await?;
        let mut entries = fs::read_dir(&source).await?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let target = destination.join(entry.file_name());
            if entry.path().is_dir() {
                stack.push((entry.path(), target));
            } else {
                fs::copy(entry.path(), &target).await?;
            }
        }
    }
    Ok(())
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the `WinUI` resources directory.
///
/// For `WinUI`, assets and fonts are placed alongside the binary in a `resources/`
/// directory. The binary locates them through the runtime's executable-relative
/// bundle lookup at startup.
/// Stage the project's assets and fonts under the backend's `resources`
/// directory. Runs after the backend build: `symbols` is the app library
/// artifact it produced, whose `waterui_meta_bundle_*` statics declare the
/// asset mounts.
async fn copy_assets_and_fonts(
    project: &Project,
    backend_path: &Path,
    symbols: &crate::artifact_symbols::ArtifactSymbols,
    dev_server: bool,
) -> eyre::Result<()> {
    let resources_dir = backend_path.join("resources");
    fs::create_dir_all(&resources_dir).await?;

    // Stage project assets using platform-native conventions.
    let manifest =
        assets::stage_project_assets_for_gtk(project, &resources_dir, symbols, dev_server).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project, &backend_path.join("Cargo.toml")).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);

    if !resolved_fonts.is_empty() {
        // Copy fonts to resources/fonts/
        let fonts_dest = resources_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;

        info!("Copied {} fonts to WinUI resources", resolved_fonts.len());
    }

    Ok(())
}
