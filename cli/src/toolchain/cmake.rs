//! Toolchain support for `CMake`.

use std::path::PathBuf;

use crate::{
    brew::Brew,
    toolchain::linux::{
        LinuxPackageManagerError, has_supported_package_manager, install_named_packages,
    },
    toolchain::managed_tool::{self, ManagedTool, ManagedToolError},
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
    /// `PATH` first, then the managed install under `~/.water/tools`.
    ///
    /// # Errors
    /// - If `CMake` is not found in the system PATH or the managed tools.
    pub async fn path(&self, host: &Host) -> Result<PathBuf, which::Error> {
        match host.which("cmake").await {
            Ok(path) => Ok(path),
            Err(error) => managed_tool::cmake()
                .and_then(|tool| tool.binary_path(host))
                .ok_or(error),
        }
    }
}

/// What a missing `cmake` on Windows resolves to: `winget` when present,
/// otherwise a pinned release archive unpacked under `~/.water/tools` — no
/// package manager required.
async fn missing_cmake_on_windows(host: &Host) -> ToolchainError<CmakeInstallation> {
    if host.which("winget").await.is_ok() {
        ToolchainError::fixable(CmakeInstallation::Winget)
    } else if let Some(tool) = managed_tool::cmake() {
        ToolchainError::fixable(CmakeInstallation::Managed(tool))
    } else {
        ToolchainError::unfixable(
            "CMake is missing and this host has no usable installer",
            "Download CMake from https://cmake.org/download/ and put `cmake` on PATH, then re-run `water doctor`.",
        )
    }
}

impl Toolchain for Cmake {
    type Installation = CmakeInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        // Check if CMake is installed
        // TODO: Also detect android-cmake toolchain files if needed
        if self.path(host).await.is_ok() {
            Ok(())
        } else if cfg!(target_os = "windows") {
            Err(missing_cmake_on_windows(host).await)
        } else if cfg!(target_os = "macos") {
            if host.which("brew").await.is_ok() {
                Err(ToolchainError::fixable(CmakeInstallation::Brew))
            } else {
                Err(ToolchainError::unfixable(
                    "CMake not found and Homebrew is unavailable",
                    "Install CMake from https://cmake.org/download/ (or Homebrew) and ensure `cmake` is available in PATH.",
                ))
            }
        } else if cfg!(target_os = "linux") {
            if has_supported_package_manager(host).await {
                Err(ToolchainError::fixable(CmakeInstallation::PackageManager))
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

/// Installation for `CMake` — the strategy `check` selected for this host.
#[derive(Debug, Clone)]
pub enum CmakeInstallation {
    /// `brew install cmake`.
    Brew,
    /// `winget install Kitware.CMake`.
    Winget,
    /// The host's Linux package manager.
    PackageManager,
    /// A pinned, checksum-verified release archive unpacked under
    /// `~/.water/tools` — no package manager required.
    Managed(ManagedTool),
}

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

    /// The managed archive install failed.
    #[error(transparent)]
    Managed(#[from] ManagedToolError),
}

impl Installation for CmakeInstallation {
    type Error = FailToInstallCmake;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        match self {
            Self::Brew => {
                let brew = Brew::default();
                brew.check(host)
                    .await
                    .map_err(|_| FailToInstallCmake::BrewNotFound)?;
                brew.install(host, "cmake").await?;
                Ok(())
            }
            Self::Winget => ensure_package_installed(host, "Kitware.CMake")
                .await
                .map_err(map_winget_error_for_cmake),
            Self::PackageManager => install_named_packages(host, &["cmake"])
                .await
                .map_err(map_linux_error_for_cmake),
            Self::Managed(tool) => {
                tool.install(host).await?;
                Ok(())
            }
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
    use super::{Cmake, CmakeInstallation, missing_cmake_on_windows};
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
        // Windows hosts have the managed-archive fallback, so a bare Windows
        // machine is fixable even without winget; elsewhere no package
        // manager means manual.
        if cfg!(target_os = "windows") && crate::toolchain::managed_tool::cmake().is_some() {
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "missing cmake on Windows without winget falls back to the managed archive: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(ToolchainError::Unfixable(_))),
                "missing cmake without a package manager must be unfixable: {result:?}"
            );
        }
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
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn install_runs_the_package_manager() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        let host = machine.host(Vec::<(String, String)>::new());
        let installation = if cfg!(target_os = "macos") {
            CmakeInstallation::Brew
        } else {
            CmakeInstallation::PackageManager
        };
        smol::block_on(installation.install(&host))
            .expect("installing cmake through the host's package manager must succeed");
    }

    /// The fake `winget` accepts `install` but never reports the package
    /// afterwards, so the post-install verification must fail fast instead of
    /// reporting success.
    #[test]
    #[cfg(target_os = "windows")]
    fn install_fails_when_winget_leaves_the_package_missing() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(CmakeInstallation::Winget.install(&host));
        assert!(
            matches!(
                result,
                Err(super::FailToInstallCmake::WingetInstallFailed(_))
            ),
            "a package still missing after winget install must be an error: {result:?}"
        );
    }

    #[test]
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    fn install_unsupported_platform() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(CmakeInstallation::PackageManager.install(&host));
        assert!(
            matches!(
                result,
                Err(super::FailToInstallCmake::UnsupportedPackageManager)
            ),
            "install on unsupported platforms must fail fast: {result:?}"
        );
    }

    /// A Windows host without `winget` gets the managed archive — fixable,
    /// never a pointer at another prerequisite installer.
    #[test]
    fn windows_host_without_winget_is_fixable_managed() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(missing_cmake_on_windows(&host));
        match crate::toolchain::managed_tool::cmake() {
            Some(_) => assert!(
                matches!(
                    result,
                    ToolchainError::Fixable(CmakeInstallation::Managed(_))
                ),
                "no winget must fall back to the managed archive: {result:?}"
            ),
            None => assert!(
                matches!(result, ToolchainError::Unfixable(_)),
                "no managed build for this architecture must be unfixable: {result:?}"
            ),
        }
    }

    #[test]
    fn windows_host_with_winget_prefers_winget() {
        let machine = TestMachine::new();
        machine.install("winget");
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(missing_cmake_on_windows(&host));
        assert!(
            matches!(result, ToolchainError::Fixable(CmakeInstallation::Winget)),
            "winget stays preferred when present: {result:?}"
        );
    }

    /// A cmake unpacked under `~/.water/tools` satisfies the check even
    /// though nothing named `cmake` is on `PATH`.
    #[test]
    fn ok_when_cmake_is_managed() {
        let machine = TestMachine::new();
        let Some(tool) = crate::toolchain::managed_tool::cmake() else {
            return; // this architecture has no managed build
        };
        let host = machine.host(Vec::<(String, String)>::new());
        let install_dir = tool.install_dir(&host).unwrap();
        machine.file(
            install_dir
                .join(&tool.binary)
                .strip_prefix(machine.root())
                .unwrap(),
            "",
        );
        let result = smol::block_on(Cmake::default().check(&host));
        assert!(
            result.is_ok(),
            "a managed cmake must satisfy the check: {result:?}"
        );
    }
}
