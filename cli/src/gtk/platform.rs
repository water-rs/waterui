//! GTK platform implementation for building and running on the local machine.

use std::path::PathBuf;

use color_eyre::eyre::{self, bail};
use target_lexicon::{DefaultToHost, Triple};

use crate::{
    build::BuildOptions,
    device::Artifact,
    gtk::{backend::GtkBackend, device::GtkDevice, toolchain::GtkToolchain},
    platform::{PackageOptions, Platform},
    project::Project,
    utils::run_command,
};

/// GTK platform for building and running on the local machine.
#[derive(Debug, Clone, Copy, Default)]
pub struct GtkPlatform;

impl GtkPlatform {
    /// Create a new GTK platform instance.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Platform for GtkPlatform {
    type Device = GtkDevice;
    type Toolchain = GtkToolchain;

    async fn scan(&self) -> eyre::Result<Vec<Self::Device>> {
        // GTK always runs on the local machine - return a single device
        Ok(vec![GtkDevice])
    }

    async fn build(&self, project: &Project, options: BuildOptions) -> eyre::Result<PathBuf> {
        let backend_path = project.backend_path::<GtkBackend>();
        let cargo_toml = backend_path.join("Cargo.toml");

        if !cargo_toml.exists() {
            bail!(
                "GTK backend not found at {}. Run 'water run --platform gtk' to initialize it.",
                backend_path.display()
            );
        }

        // Build the GTK binary crate
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
        // GTK uses its own target directory since it's a standalone project
        let target_dir = backend_path.join("target").join(profile);
        Ok(target_dir)
    }

    fn toolchain(&self) -> Self::Toolchain {
        GtkToolchain
    }

    fn triple(&self) -> Triple {
        // GTK runs on the host machine
        DefaultToHost::default().0
    }

    async fn clean(&self, project: &Project) -> eyre::Result<()> {
        let backend_path = project.backend_path::<GtkBackend>();
        let cargo_toml = backend_path.join("Cargo.toml");

        if !cargo_toml.exists() {
            return Ok(()); // Nothing to clean
        }

        // Run cargo clean for the GTK crate
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

    async fn package(&self, project: &Project, options: PackageOptions) -> eyre::Result<Artifact> {
        // For GTK, "packaging" just means locating the built binary
        let profile = if options.is_debug() { "debug" } else { "release" };
        // GTK uses its own target directory since it's a standalone project
        let backend_path = project.backend_path::<GtkBackend>();
        let target_dir = backend_path.join("target").join(profile);

        // The binary name is the GTK crate name (project-gtk)
        let crate_name = project.crate_name();
        let binary_name = format!("{crate_name}-gtk");

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
                "Built GTK binary not found at {}. Did you run build first?",
                binary_path.display()
            );
        }

        Ok(Artifact::new(project.bundle_identifier(), binary_path))
    }
}
