//! Toolchain support for `CMake`.

use std::path::PathBuf;

use color_eyre::eyre;

use crate::{
    brew::Brew,
    toolchain::{Installation, Toolchain},
    utils::which,
};

/// Toolchain for `CMake`
#[derive(Debug, Clone, Default)]
pub struct Cmake {}

impl Cmake {
    /// Get the path to the `cmake` executable.
    ///
    /// # Errors
    /// - If `CMake` is not found in the system PATH.
    pub async fn path(&self) -> eyre::Result<PathBuf> {
        which("cmake").await.map_err(|e| eyre::eyre!(e))
    }
}

impl Toolchain for Cmake {
    type Installation = CmakeInstallation;

    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        // Check if CMake is installed
        // TODO: Also detect android-cmake toolchain files if needed
        if which("cmake").await.is_ok() {
            Ok(())
        } else {
            Err(crate::toolchain::ToolchainError::fixable(CmakeInstallation))
        }
    }
}

/// Installation for `CMake`
#[derive(Debug, Clone)]
pub struct CmakeInstallation;

/// Errors that can occur during `CMake` installation
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallCmake {
    /// Homebrew not found error
    #[error("Homebrew not found. Please install Homebrew to proceed.")]
    BrewNotFound,

    /// Other installation errors
    #[error("Failed to install CMake via Homebrew: {0}")]
    Other(eyre::Report),

    /// Unsupported platform error
    #[error(
        "Automatic installation of CMake is not supported on this platform. Please install CMake manually."
    )]
    UnsupportedPlatform,
}

impl Installation for CmakeInstallation {
    type Error = FailToInstallCmake;

    async fn install(&self) -> Result<(), Self::Error> {
        if cfg!(target_os = "macos") {
            let brew = Brew::default();

            brew.check()
                .await
                .map_err(|_| FailToInstallCmake::BrewNotFound)?;
            brew.install("cmake")
                .await
                .map_err(FailToInstallCmake::Other)?;

            Ok(())
        } else {
            Err(FailToInstallCmake::UnsupportedPlatform)
        }
    }
}
