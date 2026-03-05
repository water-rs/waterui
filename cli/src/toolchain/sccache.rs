//! Toolchain support for `sccache` - shared compilation cache.

use std::path::PathBuf;

use color_eyre::eyre;

use crate::{
    brew::Brew,
    toolchain::linux::{has_supported_package_manager, install_named_packages},
    toolchain::winget::{WingetInstallError, ensure_package_installed},
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::{sccache_install_hint, which},
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
        } else if cfg!(target_os = "windows") {
            if which("winget").await.is_ok() {
                Err(ToolchainError::fixable(SccacheInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "sccache not found and winget is unavailable",
                    format!(
                        "Install Microsoft App Installer to provide winget, or install manually with {}.",
                        sccache_install_hint()
                    ),
                ))
            }
        } else if cfg!(target_os = "macos") {
            if which("brew").await.is_ok() {
                Err(ToolchainError::fixable(SccacheInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "sccache not found and Homebrew is unavailable",
                    format!(
                        "Install Homebrew to enable automatic fixes, or install manually with {}.",
                        sccache_install_hint()
                    ),
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager().await {
                Err(ToolchainError::fixable(SccacheInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "sccache is missing and no supported package manager was found",
                    format!("Install manually with {}", sccache_install_hint()),
                ))
            }
        } else {
            Err(ToolchainError::unfixable(
                "sccache not found",
                format!(
                    "Install sccache manually ({}) and ensure `sccache` is available in PATH.",
                    sccache_install_hint()
                ),
            ))
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
    #[error("Failed to install sccache: {0}")]
    Other(eyre::Report),

    /// winget is required for Windows automatic installation.
    #[error(
        "winget is required for automatic sccache installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,

    /// Windows installation via winget failed.
    #[error("Failed to install sccache via winget: {0}")]
    WingetInstallFailed(String),

    /// Linux package manager is required for automatic installation.
    #[error(
        "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk). Install sccache manually."
    )]
    UnsupportedPackageManager,

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
        } else if cfg!(target_os = "windows") {
            ensure_package_installed("Mozilla.sccache")
                .await
                .map_err(map_winget_error_for_sccache)
        } else if cfg!(target_os = "linux") {
            install_named_packages(&["sccache"])
                .await
                .map_err(map_linux_error_for_sccache)
        } else {
            Err(FailToInstallSccache::UnsupportedPlatform)
        }
    }
}

fn map_linux_error_for_sccache(error: eyre::Report) -> FailToInstallSccache {
    let message = error.to_string();
    if message.contains("No supported Linux package manager found") {
        FailToInstallSccache::UnsupportedPackageManager
    } else {
        FailToInstallSccache::Other(error)
    }
}

fn map_winget_error_for_sccache(error: WingetInstallError) -> FailToInstallSccache {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallSccache::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallSccache::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallSccache::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FailToInstallSccache, map_winget_error_for_sccache};
    use crate::toolchain::winget::WingetInstallError;

    #[test]
    fn maps_winget_not_found_to_specific_error() {
        let mapped = map_winget_error_for_sccache(WingetInstallError::WingetNotFound);
        assert!(matches!(mapped, FailToInstallSccache::WingetNotFound));
    }

    #[test]
    fn maps_not_installed_error_with_package_context() {
        let mapped = map_winget_error_for_sccache(WingetInstallError::NotInstalled {
            package_id: "Mozilla.sccache",
        });
        let message = mapped.to_string();
        assert!(message.contains("Mozilla.sccache"));
        assert!(message.contains("still missing"));
    }
}
