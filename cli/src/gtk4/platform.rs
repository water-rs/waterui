//! GTK4 platform build and package utilities.
//!
//! This module provides utility functions for building and packaging GTK4 apps.
//! These functions are used by `Gtk4Backend` to implement the `Backend` trait.

use std::ffi::OsString;
use std::path::Path;

use eyre::bail;
use smol::fs;
use tracing::info;

use crate::{
    assets, browser_runtime,
    build::{BuildOptions, BuiltTarget, RustBuild, RustDynamicLibraries, RustLinkage},
    device::Artifact,
    gtk4::backend::Gtk4Backend,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::run_command_os,
};

#[cfg(target_os = "linux")]
const GTK4_INIT_HINT: &str = "water run --platform linux --backend gtk4";
#[cfg(not(target_os = "linux"))]
const GTK4_INIT_HINT: &str = "initialize GTK4 backend on Linux";

// ============================================================================
// Build Utilities
// ============================================================================

/// Build GTK4 binary for the host platform.
///
/// # Errors
/// Returns an error if the backend manifest is missing, the host is unsupported, or Cargo fails.
pub async fn build_gtk4(project: &Project, options: BuildOptions) -> eyre::Result<BuiltTarget> {
    ensure_linux_host()?;

    let backend_path = project.backend_path::<Gtk4Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        bail!(
            "GTK4 backend not found at {}. Run `{GTK4_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    let mut build = RustBuild::new(&backend_path, TargetPlatform::Linux.triple())
        .with_project(project)
        .with_target_dir(project.water_target_dir(options.linkage()).await?)
        .with_linkage(
            options.linkage(),
            &format!("{}/dev", project.crate_name()),
            &["$ORIGIN"],
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
            project.gtk_backend_crate_name().as_str(),
            options.is_release(),
        )
        .await
        .map_err(|error| eyre::eyre!("Failed to build GTK4 backend with cargo: {error}"))?;
    Ok(built_target)
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Cargo build artifacts for GTK4.
///
/// # Errors
/// Returns an error if the host is unsupported or `cargo clean` fails.
pub async fn clean_gtk4(project: &Project) -> eyre::Result<()> {
    ensure_linux_host()?;

    let backend_path = project.backend_path::<Gtk4Backend>();
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
            project.gtk_backend_crate_name().as_str().into(),
        ];
        run_command_os("cargo", args).await?;
    }

    Ok(())
}

// ============================================================================
// Package
// ============================================================================

/// Package a GTK4 app (locate the built binary).
///
/// # Errors
/// Returns an error if the host is unsupported, assets cannot be staged, or the built binary is missing.
pub async fn package_gtk4(
    project: &Project,
    options: PackageOptions,
    built: &BuiltTarget,
) -> eyre::Result<Artifact> {
    ensure_linux_host()?;

    // For GTK4, "packaging" just means locating the built binary
    // GTK4 uses its own target directory since it's a standalone project
    let backend_path = project.backend_path::<Gtk4Backend>();

    // Copy project assets and dependency fonts
    copy_assets_and_fonts(
        project,
        &backend_path,
        &built.app_symbols()?,
        options.uses_dev_server(),
    )
    .await?;

    let target_dir = &built.profile_dir;
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };

    // The binary name is the GTK4 crate name (project-gtk4)
    let final_binary_path = &built.artifact;
    let runtime_plan = project
        .browser_runtime_plan(TargetPlatform::Linux, crate::platform::TargetBackend::Gtk4)
        .await?;

    // The shipped binary and everything `$ORIGIN` resolves beside it stage
    // into the project's own managed backend directory — the shared Cargo
    // profile directory would collide two same-named projects on
    // `<profile>/<product>`.
    let runtime_dir =
        crate::platforming::packaging::dist_dir(&backend_path, "linux", Some(profile));
    fs::create_dir_all(&runtime_dir).await?;
    browser_runtime::stage(
        runtime_plan,
        TargetPlatform::Linux,
        target_dir,
        &runtime_dir,
    )
    .await?;

    if options.uses_shared_rust_runtime() {
        RustDynamicLibraries::resolve(built, &TargetPlatform::Linux.triple(), project)
            .await?
            .stage(&runtime_dir)
            .await?;
    } else {
        RustDynamicLibraries::remove_staged(&runtime_dir, &TargetPlatform::Linux.triple()).await?;
    }

    // Ship the binary under the product name; the tagged Cargo artifact name
    // is internal to the shared target directory.
    let packaged_binary = crate::platforming::packaging::stage_binary_as(
        final_binary_path,
        &runtime_dir,
        project.gtk4_binary_name().as_str(),
    )
    .await?;

    Ok(Artifact::new(project.bundle_identifier(), packaged_binary))
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the GTK4 backend.
#[must_use]
pub const fn is_gtk4_platform(platform: TargetPlatform) -> bool {
    matches!(platform, TargetPlatform::Linux)
}

fn ensure_linux_host() -> eyre::Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!("GTK4 backend is only supported on Linux hosts");
    }
}

// ============================================================================
// Asset and Font Handling
// ============================================================================

/// Copy project assets and dependency fonts to the GTK4 resources directory.
///
/// For GTK4, assets and fonts are placed alongside the binary in a `resources/`
/// directory. The binary should load fonts via fontconfig or Pango at runtime.
/// Stage the project's assets and fonts under the backend's `resources`
/// directory. `symbols` is the app library artifact the target build
/// produced, whose `waterui_meta_bundle_*` statics declare the asset mounts.
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
    assets::stage_hicolor_icons(project, &resources_dir.join("icons")).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project, &backend_path.join("Cargo.toml")).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(&manifest)?);

    if !resolved_fonts.is_empty() {
        // Copy fonts to resources/fonts/
        let fonts_dest = resources_dir.join("fonts");
        assets::copy_fonts(&resolved_fonts, &fonts_dest).await?;

        info!("Copied {} fonts to GTK4 resources", resolved_fonts.len());

        // Note: GTK4 font registration happens at runtime via fontconfig/pango.
        // The hydrolysis backend should register fonts from the resources/fonts directory
        // when initializing.
    }

    Ok(())
}
