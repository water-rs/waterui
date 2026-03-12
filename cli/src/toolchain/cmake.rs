//! Toolchain support for `CMake`.

use std::path::PathBuf;

use color_eyre::eyre;

use crate::{
    brew::Brew,
    toolchain::linux::{has_supported_package_manager, install_named_packages},
    toolchain::winget::{WingetInstallError, ensure_package_installed},
    toolchain::{Installation, Toolchain, ToolchainError},
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

    async fn check(&self) -> Result<(), ToolchainError<Self::Installation>> {
        // Check if CMake is installed
        // TODO: Also detect android-cmake toolchain files if needed
        if which("cmake").await.is_ok() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            if which("winget").await.is_ok() {
                Err(ToolchainError::fixable(CmakeInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "CMake not found and winget is unavailable",
                    "Install Microsoft App Installer to provide winget, or install CMake manually and ensure `cmake` is available in PATH.",
                ))
            }
        } else if cfg!(target_os = "macos") {
            if which("brew").await.is_ok() {
                Err(ToolchainError::fixable(CmakeInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "CMake not found and Homebrew is unavailable",
                    "Install Homebrew to enable automatic fixes, or install CMake manually and ensure `cmake` is available in PATH.",
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager().await {
                Err(ToolchainError::fixable(CmakeInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "CMake is missing and no supported package manager was found",
                    "Install CMake manually and ensure `cmake` is available in PATH.",
                ))
            }
        } else {
            Err(ToolchainError::unfixable(
                "CMake not found",
                "Install CMake manually for your platform and ensure `cmake` is available in PATH.",
            ))
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

    /// Other installation errors.
    #[error("Failed to install CMake: {0}")]
    Other(eyre::Report),

    /// winget is required for Windows automatic installation.
    #[error(
        "winget is required for automatic CMake installation on Windows. Install App Installer and retry."
    )]
    WingetNotFound,

    /// Windows installation via winget failed.
    #[error("Failed to install CMake via winget: {0}")]
    WingetInstallFailed(String),

    /// Linux package manager is required for automatic installation.
    #[error(
        "No supported Linux package manager found (apt-get, dnf, pacman, zypper, apk). Install CMake manually."
    )]
    UnsupportedPackageManager,

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
        } else if cfg!(target_os = "windows") {
            ensure_package_installed("Kitware.CMake")
                .await
                .map_err(map_winget_error_for_cmake)
        } else if cfg!(target_os = "linux") {
            install_named_packages(&["cmake"])
                .await
                .map_err(map_linux_error_for_cmake)
        } else {
            Err(FailToInstallCmake::UnsupportedPlatform)
        }
    }
}

fn map_linux_error_for_cmake(error: eyre::Report) -> FailToInstallCmake {
    let message = error.to_string();
    if message.contains("No supported Linux package manager found") {
        FailToInstallCmake::UnsupportedPackageManager
    } else {
        FailToInstallCmake::Other(error)
    }
}

fn map_winget_error_for_cmake(error: WingetInstallError) -> FailToInstallCmake {
    match error {
        WingetInstallError::WingetNotFound => FailToInstallCmake::WingetNotFound,
        WingetInstallError::CommandFailed(err) => {
            FailToInstallCmake::WingetInstallFailed(err.to_string())
        }
        WingetInstallError::NotInstalled { package_id } => {
            FailToInstallCmake::WingetInstallFailed(format!(
                "Package `{package_id}` is still missing after winget install; verify winget sources and retry."
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FailToInstallCmake, map_winget_error_for_cmake};
    use crate::toolchain::winget::WingetInstallError;

    #[test]
    fn maps_winget_not_found_to_specific_error() {
        let mapped = map_winget_error_for_cmake(WingetInstallError::WingetNotFound);
        assert!(matches!(mapped, FailToInstallCmake::WingetNotFound));
    }

    #[test]
    fn maps_not_installed_error_with_package_context() {
        let mapped = map_winget_error_for_cmake(WingetInstallError::NotInstalled {
            package_id: "Kitware.CMake",
        });
        let message = mapped.to_string();
        assert!(message.contains("Kitware.CMake"));
        assert!(message.contains("still missing"));
    }
}
