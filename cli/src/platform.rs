use std::path::PathBuf;

use color_eyre::eyre;
use target_lexicon::Triple;

use crate::{build::BuildOptions, device::Device, project::Project, toolchain::Toolchain};

/// Build options for the platform
#[derive(Debug, Clone)]
pub struct PackageOptions {
    release: bool,
}

impl PackageOptions {
    /// Create new package options
    #[must_use]
    pub const fn new(release: bool) -> Self {
        Self { release }
    }

    /// Whether to build in release mode
    #[must_use]
    pub const fn is_release(&self) -> bool {
        self.release
    }
}

/// Trait representing a specific platform (e.g., Android, iOS, etc.)
pub trait Platform: Send {
    /// The associated toolchain type for this platform.
    type Toolchain: Toolchain;
    /// The associated device type for this platform.
    type Device: Device;

    /// Clean build artifacts for this platform (not include rust build artifacts)
    fn clean(&self, project: &Project) -> impl Future<Output = eyre::Result<()>> + Send;

    /// Package the project for this platform
    ///
    /// Return the path to the packaged file
    fn package(
        &self,
        project: &Project,
        options: PackageOptions,
    ) -> impl Future<Output = eyre::Result<PathBuf>> + Send;

    /// Get the toolchain for this platform
    fn toolchain(&self) -> &Self::Toolchain;

    /// Scan for connected devices for this platform
    fn scan(&self) -> impl Future<Output = eyre::Result<Vec<Self::Device>>> + Send;

    fn triple(&self) -> Triple;

    /// Build the Rust library for this platform
    ///
    /// Warning: This method would build and copy the built library to the appropriate location in the project directory.
    /// However, it does not handle hot reload library building or management.
    ///
    /// Return the path to the built library
    fn build(&self, options: BuildOptions) -> impl Future<Output = eyre::Result<PathBuf>> + Send;
}
