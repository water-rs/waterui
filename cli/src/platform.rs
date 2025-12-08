use std::path::PathBuf;

use color_eyre::eyre;
use target_lexicon::Triple;

use crate::{
    build::BuildOptions,
    device::{Artifact, Device},
    project::Project,
    toolchain::Toolchain,
};

/// Build options for the platform
#[derive(Debug, Clone)]
/// Configuration options for packaging the application.
///
/// This struct contains settings that control how the application
/// is packaged for distribution across different platforms.
pub struct PackageOptions {
    /// Whether to prepare the package for store distribution.
    ///
    /// When `true`, the package will be configured for submission to
    /// official app stores (App Store for iOS/macOS or Play Store for Android).
    ///
    /// When `false`, the package will be prepared for direct distribution
    /// or development purposes.
    distribution: bool,
}

impl PackageOptions {
    /// Create new package options
    #[must_use]
    pub const fn new(distribution: bool) -> Self {
        Self { distribution }
    }

    /// Whether to package in distribution mode
    #[must_use]
    pub const fn is_distribution(&self) -> bool {
        self.distribution
    }
}

/// Trait representing a specific platform (e.g., Android, iOS, etc.)
///
/// Note: `Platform` would never check toolchain since it is the responsibility of the `Toolchain`.
/// We assume the toolchain is already set up correctly when calling methods of this trait.
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
    ///
    /// # Warnings
    /// You should call `build` before calling this method to ensure the project is built for the platform.
    fn package(
        &self,
        project: &Project,
        options: PackageOptions,
    ) -> impl Future<Output = eyre::Result<Artifact>> + Send;

    /// Get the toolchain for this platform
    fn toolchain(&self) -> Self::Toolchain;

    /// Scan for connected devices for this platform
    fn scan(&self) -> impl Future<Output = eyre::Result<Vec<Self::Device>>> + Send;

    fn triple(&self) -> Triple;

    /// Build the Rust library for this platform
    ///
    /// Warning: This method would build and copy the built library to the appropriate location in the project directory.
    /// However, it does not handle hot reload library building or management.
    ///
    /// Return the target directory path where the built library is located.
    fn build(
        &self,
        project: &Project,
        options: BuildOptions,
    ) -> impl Future<Output = eyre::Result<PathBuf>> + Send;
}
