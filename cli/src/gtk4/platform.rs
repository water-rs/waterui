//! GTK4 platform build and package utilities.
//!
//! This module provides utility functions for building and packaging GTK4 apps.
//! These functions are used by `Gtk4Backend` to implement the `Backend` trait.

use std::path::PathBuf;

use color_eyre::eyre::{self, bail};

use crate::{
    build::BuildOptions,
    device::Artifact,
    gtk4::backend::Gtk4Backend,
    platform::{PackageOptions, TargetPlatform},
    project::Project,
    utils::run_command,
};

// ============================================================================
// Build Utilities
// ============================================================================

/// Build GTK4 binary for the host platform.
pub async fn build_gtk4(project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
    let backend_path = project.backend_path::<Gtk4Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        bail!(
            "GTK4 backend not found at {}. Run 'water run --platform gtk4' to initialize it.",
            backend_path.display()
        );
    }

    // Build the GTK4 binary crate
    let profile = if options.is_release() {
        "release"
    } else {
        "debug"
    };

    let mut args = vec!["build", "--manifest-path"];
    let cargo_toml_str = cargo_toml.to_string_lossy().to_string();
    args.push(&cargo_toml_str);

    if options.is_release() {
        args.push("--release");
    }

    run_command("cargo", args.iter().copied()).await?;

    // Return the target directory where the binary was built
    // GTK4 uses its own target directory since it's a standalone project
    let target_dir = backend_path.join("target").join(profile);
    Ok(target_dir)
}

// ============================================================================
// Clean
// ============================================================================

/// Clean Cargo build artifacts for GTK4.
pub async fn clean_gtk4(project: &Project) -> eyre::Result<()> {
    let backend_path = project.backend_path::<Gtk4Backend>();
    let cargo_toml = backend_path.join("Cargo.toml");

    if !cargo_toml.exists() {
        return Ok(()); // Nothing to clean
    }

    // Run cargo clean for the GTK4 crate
    run_command(
        "cargo",
        [
            "clean",
            "--manifest-path",
            cargo_toml.to_str().unwrap_or_default(),
        ],
    )
    .await?;

    Ok(())
}

// ============================================================================
// Package
// ============================================================================

/// Package a GTK4 app (locate the built binary).
pub async fn package_gtk4(project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
    // For GTK4, "packaging" just means locating the built binary
    let profile = if options.is_debug() { "debug" } else { "release" };
    // GTK4 uses its own target directory since it's a standalone project
    let backend_path = project.backend_path::<Gtk4Backend>();
    let target_dir = backend_path.join("target").join(profile);

    // The binary name is the GTK4 crate name (project-gtk4)
    let crate_name = project.crate_name();
    let binary_name = format!("{crate_name}-gtk4");

    // Handle platform-specific binary extension
    let binary_path = if cfg!(windows) {
        target_dir.join(format!("{binary_name}.exe"))
    } else {
        target_dir.join(&binary_name)
    };

    if !binary_path.exists() {
        // Try to find the binary by checking if it's using a different naming convention
        let alt_binary_name = binary_name.replace('-', "_");
        let alt_binary_path = if cfg!(windows) {
            target_dir.join(format!("{alt_binary_name}.exe"))
        } else {
            target_dir.join(&alt_binary_name)
        };

        if alt_binary_path.exists() {
            return Ok(Artifact::new(
                project.bundle_identifier(),
                alt_binary_path,
            ));
        }

        bail!(
            "Built GTK4 binary not found at {}. Did you run build first?",
            binary_path.display()
        );
    }

    Ok(Artifact::new(project.bundle_identifier(), binary_path))
}

// ============================================================================
// Platform Support Check
// ============================================================================

/// Check if a platform is supported by the GTK4 backend.
pub const fn is_gtk4_platform(platform: TargetPlatform) -> bool {
    matches!(
        platform,
        TargetPlatform::Linux | TargetPlatform::Windows | TargetPlatform::MacOS
    )
}
