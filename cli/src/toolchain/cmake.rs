//! Toolchain support for `CMake`.

use std::path::PathBuf;

use crate::{
    brew::Brew,
    toolchain::linux::{
        LinuxPackageManagerError, has_supported_package_manager, install_named_packages,
    },
    toolchain::winget::{WingetInstallError, ensure_package_installed},
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// Toolchain for `CMake`
#[derive(Debug, Clone, Default)]
pub struct Cmake {}

impl Cmake {
    /// Get the path to the `cmake` executable.
    ///
    /// # Errors
    /// - If `CMake` is not found in the system PATH.
    pub async fn path(&self, host: &Host) -> Result<PathBuf, which::Error> {
        host.which("cmake").await
    }
}

impl Toolchain for Cmake {
    type Installation = CmakeInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        // Check if CMake is installed
        // TODO: Also detect android-cmake toolchain files if needed
        if host.which("cmake").await.is_ok() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            if host.which("winget").await.is_ok() {
                Err(ToolchainError::fixable(CmakeInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "CMake not found and winget is unavailable",
                    "Install Microsoft App Installer to provide winget, or install CMake manually and ensure `cmake` is available in PATH.",
                ))
            }
        } else if cfg!(target_os = "macos") {
            if host.which("brew").await.is_ok() {
                Err(ToolchainError::fixable(CmakeInstallation))
            } else {
                Err(ToolchainError::unfixable(
                    "CMake not found and Homebrew is unavailable",
                    "Install Homebrew to enable automatic fixes, or install CMake manually and ensure `cmake` is available in PATH.",
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager(host).await {
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

    /// An installation command failed.
    #[error("Failed to install CMake: {0}")]
    Command(#[from] CommandError),

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

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if cfg!(target_os = "macos") {
            let brew = Brew::default();

            brew.check(host)
                .await
                .map_err(|_| FailToInstallCmake::BrewNotFound)?;
            brew.install(host, "cmake").await?;

            Ok(())
        } else if cfg!(target_os = "windows") {
            ensure_package_installed(host, "Kitware.CMake")
                .await
                .map_err(map_winget_error_for_cmake)
        } else if cfg!(target_os = "linux") {
            install_named_packages(host, &["cmake"])
                .await
                .map_err(map_linux_error_for_cmake)
        } else {
            Err(FailToInstallCmake::UnsupportedPlatform)
        }
    }
}

fn map_linux_error_for_cmake(error: LinuxPackageManagerError) -> FailToInstallCmake {
    match error {
        LinuxPackageManagerError::UnsupportedPackageManager => {
            FailToInstallCmake::UnsupportedPackageManager
        }
        LinuxPackageManagerError::Command(source) => FailToInstallCmake::Command(source),
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
mod host_tests {
    use super::{Cmake, CmakeInstallation};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Installation, Toolchain, ToolchainError};

    fn check(machine: &TestMachine) -> Result<(), ToolchainError<CmakeInstallation>> {
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Cmake::default().check(&host))
    }

    #[test]
    fn ok_when_cmake_on_path() {
        let machine = TestMachine::new();
        machine.install("cmake");
        check(&machine).expect("cmake on PATH must be ok");
    }

    #[test]
    fn missing_without_installer_is_unfixable() {
        let machine = TestMachine::new();
        let result = check(&machine);
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "missing cmake without a package manager must be unfixable: {result:?}"
        );
    }

    #[test]
    fn missing_with_installer_is_fixable() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        #[cfg(target_os = "windows")]
        machine.install("winget");
        let result = check(&machine);
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing cmake with a package manager must be fixable: {result:?}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn install_runs_brew() {
        let machine = TestMachine::new();
        machine.install("brew");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(CmakeInstallation.install(&host))
            .expect("brew install cmake must succeed on a host that provides brew");
    }

    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn install_unsupported_platform() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(CmakeInstallation.install(&host));
        assert!(
            matches!(result, Err(super::FailToInstallCmake::UnsupportedPlatform)),
            "install on unsupported platforms must fail fast: {result:?}"
        );
    }
}
