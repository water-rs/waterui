//! Toolchain support for `meson`.

use std::path::PathBuf;

use color_eyre::eyre;

use crate::{
    brew::Brew,
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::which,
};

/// Toolchain for `meson`.
#[derive(Debug, Clone, Default)]
pub struct Meson;

impl Meson {
    /// Get the path to the `meson` executable.
    ///
    /// # Errors
    /// Returns an error if `meson` is not found in PATH.
    pub async fn path(&self) -> eyre::Result<PathBuf> {
        which("meson").await.map_err(|e| eyre::eyre!(e))
    }
}

impl Toolchain for Meson {
    type Installation = MesonInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if which("meson").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::fixable(MesonInstallation))
        }
    }
}

/// Installation plan for `meson`.
#[derive(Debug, Clone)]
pub struct MesonInstallation;

/// Errors that can occur during `meson` installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallMeson {
    /// Homebrew not found error.
    #[error("Homebrew not found. Please install Homebrew to proceed.")]
    BrewNotFound,
    /// Other installation errors.
    #[error("Failed to install meson via Homebrew: {0}")]
    Other(eyre::Report),
    /// Unsupported platform error.
    #[error(
        "Automatic installation of meson is not supported on this platform. Please install meson manually."
    )]
    UnsupportedPlatform,
}

impl Installation for MesonInstallation {
    type Error = FailToInstallMeson;

    async fn install(&self) -> Result<(), Self::Error> {
        if cfg!(target_os = "macos") {
            let brew = Brew::default();
            brew.check()
                .await
                .map_err(|_| FailToInstallMeson::BrewNotFound)?;
            brew.install("meson")
                .await
                .map_err(FailToInstallMeson::Other)?;
            Ok(())
        } else {
            Err(FailToInstallMeson::UnsupportedPlatform)
        }
    }
}
