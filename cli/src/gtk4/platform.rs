//! GTK4 platform build and package utilities.
//!
//! This module provides utility functions for building and packaging GTK4 apps.
//! These functions are used by `Gtk4Backend` to implement the `Backend` trait.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use color_eyre::eyre::{self, bail};
use smol::fs;
use tracing::info;

use crate::{
    assets, browser_runtime,
    build::{
        BuildOptions, RustDynamicLibraries, RustLinkage, configure_cargo_linkage,
        configure_generated_crate_compilation, shared_rust_runtime_fingerprint,
    },
    device::Artifact,
    gtk4::backend::Gtk4Backend,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::{command, run_command_os},
};

#[cfg(target_os = "linux")]
const GTK4_INIT_HINT: &str = "water run --platform linux --backend gtk4";
#[cfg(not(target_os = "linux"))]
const GTK4_INIT_HINT: &str = "initialize GTK4 backend on Linux";

// ============================================================================
// Build Utilities
// ============================================================================

/// Resolve the Cargo target directory the GTK4 backend crate builds into.
///
/// Build, clean and package must agree on this path, so all three go through here.
/// A shared-runtime build resolves a directory keyed by its runtime graph, which
/// projects with the same graph then share instead of each compiling their own copy.
async fn gtk4_target_dir(
    project: &Project,
    cargo_toml: &Path,
    linkage: RustLinkage,
) -> eyre::Result<PathBuf> {
    let runtime_fingerprint = match linkage {
        RustLinkage::Static => None,
        RustLinkage::SharedRuntime => {
            let build_features = vec![format!("{}/dev", project.crate_name())];
            Some(
                shared_rust_runtime_fingerprint(
                    cargo_toml,
                    &build_features,
                    &TargetPlatform::Linux.triple(),
                )
                .await?,
            )
        }
    };
    project
        .backend_build_target_dir("gtk4", runtime_fingerprint.as_deref())
        .await
}

/// Build GTK4 binary for the host platform.
///
/// # Errors
/// Returns an error if the backend manifest is missing, the host is unsupported, or Cargo fails.
pub async fn build_gtk4(project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
    ensure_linux_host()?;

    let backend_path = project.backend_path::<Gtk4Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        bail!(
            "GTK4 backend not found at {}. Run `{GTK4_INIT_HINT}` to initialize it.",
            backend_path.display(),
        );
    }

    let backend_target_dir = gtk4_target_dir(project, &cargo_toml, options.linkage()).await?;

    // Build the GTK4 binary crate.
    let profile = if options.is_release() {
        "release"
    } else {
        "debug"
    };

    let mut cargo = smol::process::Command::new("cargo");
    let cargo = command(&mut cargo);
    cargo.arg("build").arg("--manifest-path").arg(&cargo_toml);
    configure_generated_crate_compilation(cargo);
    cargo.arg("--target-dir").arg(&backend_target_dir);
    configure_cargo_linkage(
        cargo,
        options.linkage(),
        &format!("{}/dev", project.crate_name()),
        Some("$ORIGIN"),
    );
    if options.is_release() {
        cargo.arg("--release");
    }

    let output = cargo.output().await?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let details = if stderr.is_empty() {
            stdout.to_string()
        } else {
            stderr.to_string()
        };
        bail!(
            "Failed to build GTK4 backend with cargo (status {}):\n{}",
            output.status,
            details
        );
    }

    // Return the target directory where the binary was built
    // GTK4 uses its own target directory since it's a standalone project
    let target_dir = backend_target_dir.join(profile);
    Ok(target_dir)
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

    for linkage in [RustLinkage::SharedRuntime, RustLinkage::Static] {
        let backend_target_dir = gtk4_target_dir(project, &cargo_toml, linkage).await?;
        let args: Vec<OsString> = vec![
            "clean".into(),
            "--manifest-path".into(),
            cargo_toml.as_os_str().to_owned(),
            "--target-dir".into(),
            backend_target_dir.as_os_str().to_owned(),
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
pub async fn package_gtk4(project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
    ensure_linux_host()?;

    // For GTK4, "packaging" just means locating the built binary
    let profile = if options.is_debug() {
        "debug"
    } else {
        "release"
    };
    // GTK4 uses its own target directory since it's a standalone project
    let backend_path = project.backend_path::<Gtk4Backend>();

    // Copy project assets and dependency fonts
    copy_assets_and_fonts(project, &backend_path).await?;

    let linkage = if options.uses_shared_rust_runtime() {
        RustLinkage::SharedRuntime
    } else {
        RustLinkage::Static
    };
    let target_dir = gtk4_target_dir(project, &backend_path.join("Cargo.toml"), linkage)
        .await?
        .join(profile);

    // The binary name is the GTK4 crate name (project-gtk4)
    let binary_name = project.gtk_backend_crate_name();

    let binary_path = target_dir.join(binary_name.as_ref());

    let final_binary_path = if binary_path.exists() {
        binary_path
    } else {
        let alt_binary_name = binary_name.replace('-', "_");
        let alt_binary_path = target_dir.join(&alt_binary_name);

        if alt_binary_path.exists() {
            alt_binary_path
        } else {
            bail!(
                "Built GTK4 binary not found at {}. Did you run build first?",
                binary_path.display()
            );
        }
    };
    let runtime_plan = project
        .browser_runtime_plan(TargetPlatform::Linux, crate::platform::TargetBackend::Gtk4)
        .await?;
    let runtime_dir = final_binary_path.parent().ok_or_else(|| {
        eyre::eyre!(
            "GTK4 binary path has no output directory: {}",
            final_binary_path.display()
        )
    })?;
    browser_runtime::stage(
        runtime_plan,
        TargetPlatform::Linux,
        &target_dir,
        runtime_dir,
    )
    .await?;

    if options.uses_shared_rust_runtime() {
        RustDynamicLibraries::resolve(runtime_dir, &TargetPlatform::Linux.triple())
            .await?
            .stage(runtime_dir)
            .await?;
    } else {
        RustDynamicLibraries::remove_staged(runtime_dir, &TargetPlatform::Linux.triple()).await?;
    }

    Ok(Artifact::new(
        project.bundle_identifier(),
        final_binary_path,
    ))
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
async fn copy_assets_and_fonts(project: &Project, backend_path: &Path) -> eyre::Result<()> {
    let resources_dir = backend_path.join("resources");
    fs::create_dir_all(&resources_dir).await?;

    // Stage project assets using platform-native conventions.
    assets::stage_project_assets_for_gtk(project, &resources_dir).await?;
    assets::stage_hicolor_icons(project, &resources_dir.join("icons")).await?;

    // Scan and resolve dependency fonts
    let font_declarations = assets::scan_fonts(project).await?;
    let mut resolved_fonts = assets::resolve_fonts(font_declarations).await?;
    resolved_fonts.extend(assets::scan_project_font_assets(project)?);

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
