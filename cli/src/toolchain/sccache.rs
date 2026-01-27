//! Toolchain support for `sccache` - shared compilation cache.

use std::path::PathBuf;

use color_eyre::eyre;

use crate::{
    brew::Brew,
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::which,
};

/// Toolchain for `sccache` - a shared compilation cache for Rust.
///
/// sccache is optional but significantly improves build times by caching
/// compiled artifacts across builds and projects.
#[derive(Debug, Clone, Default)]
pub struct Sccache;

impl Sccache {
    /// Get the path to the `sccache` executable if available.
    ///
    /// # Errors
    /// Returns an error if `sccache` is not found in the system PATH.
    pub async fn path(&self) -> eyre::Result<PathBuf> {
        which("sccache").await.map_err(|e| eyre::eyre!(e))
    }

    /// Check if sccache is available without returning an error.
    pub async fn is_available(&self) -> bool {
        self.path().await.is_ok()
    }
}

impl Toolchain for Sccache {
    type Installation = SccacheInstallation;

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        if which("sccache").await.is_ok() {
            Ok(())
        } else {
            Err(ToolchainError::fixable(SccacheInstallation))
        }
    }
}

/// Installation plan for `sccache`.
#[derive(Debug, Clone)]
pub struct SccacheInstallation;

/// Errors that can occur during `sccache` installation.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallSccache {
    /// Homebrew not found error.
    #[error("Homebrew not found. Please install Homebrew to proceed.")]
    BrewNotFound,

    /// Other installation errors.
    #[error("Failed to install sccache via Homebrew: {0}")]
    Other(eyre::Report),

    /// Unsupported platform error.
    #[error(
        "Automatic installation of sccache is not supported on this platform. \
         Install manually with: cargo install sccache"
    )]
    UnsupportedPlatform,
}

impl Installation for SccacheInstallation {
    type Error = FailToInstallSccache;

    async fn install(&self) -> Result<(), Self::Error> {
        if cfg!(target_os = "macos") {
            let brew = Brew::default();

            brew.check()
                .await
                .map_err(|_| FailToInstallSccache::BrewNotFound)?;
            brew.install("sccache")
                .await
                .map_err(FailToInstallSccache::Other)?;

            Ok(())
        } else {
            Err(FailToInstallSccache::UnsupportedPlatform)
        }
    }
}
