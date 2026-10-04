//! Toolchain support for `spirv-opt` (SPIRV-Tools).
//!
//! `cherenkov-gpu`'s build script precompiles the engine's shaders to SPIR-V
//! and optimizes them with `spirv-opt` on every non-Apple native target —
//! Linux, Windows and Android (Apple targets compile to Metal instead). The
//! binary ships in the `spirv-tools` package every supported package manager
//! carries, alongside the `spirv-val` the build script invokes next, so a
//! PATH probe for `spirv-opt` covers the whole requirement.

use std::path::{Path, PathBuf};

use crate::{
    brew::Brew,
    toolchain::linux::{FailToInstallLinuxSystemPackages, LinuxSystemPackagesInstallation},
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// Toolchain for `spirv-opt`, the SPIR-V optimizer `cherenkov-gpu` builds invoke.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpirvTools;

impl SpirvTools {
    /// The `spirv-opt` executable on `host`'s `PATH`.
    ///
    /// # Errors
    /// Returns [`which::Error`] when no `spirv-opt` is on PATH.
    pub async fn path(&self, host: &Host) -> Result<PathBuf, which::Error> {
        host.which("spirv-opt").await
    }

    /// The first line of `spirv-opt --version` on `host`, when the binary is
    /// on `PATH` and runs cleanly.
    pub async fn version(&self, host: &Host) -> Option<String> {
        let path = self.path(host).await.ok()?;
        let version = version_of(host, &path).await.ok()?;
        if version.is_empty() {
            None
        } else {
            Some(version)
        }
    }
}

/// `spirv-opt --version`'s first line, or why it did not run.
async fn version_of(host: &Host, path: &Path) -> Result<String, CommandError> {
    let output = host.run(path, ["--version"]).await?;
    Ok(output.lines().next().unwrap_or_default().trim().to_string())
}

/// The manual install hint for this OS — a `spirv-opt` exists everywhere
/// SPIRV-Tools can be packaged, only the installer differs.
const fn manual_install_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "Install SPIRV-Tools with `brew install spirv-tools`, then re-run `water doctor`."
    } else if cfg!(target_os = "linux") {
        "Install the `spirv-tools` package with your distribution's package manager (`sudo apt-get install spirv-tools` on Debian/Ubuntu), then re-run `water doctor`."
    } else if cfg!(target_os = "windows") {
        "In an MSYS2 UCRT64 shell run `pacman -S mingw-w64-ucrt-x86_64-spirv-tools`, then add `<msys2>\\ucrt64\\bin` to PATH."
    } else {
        "Install SPIRV-Tools for your platform and ensure `spirv-opt` is on PATH."
    }
}

/// What a missing `spirv-opt` resolves to on this host: the platform's own
/// package manager where one carries `spirv-tools`, else the manual hint.
async fn missing(host: &Host) -> ToolchainError<SpirvToolsInstallation> {
    if cfg!(target_os = "macos") {
        if host.which("brew").await.is_ok() {
            ToolchainError::fixable(SpirvToolsInstallation::Brew)
        } else {
            ToolchainError::unfixable(
                "spirv-opt (SPIRV-Tools) is missing and Homebrew is unavailable",
                manual_install_hint(),
            )
        }
    } else if cfg!(target_os = "linux") {
        LinuxSystemPackagesInstallation::from_packages(host, vec!["spirv-tools".to_string()])
            .await
            .map_or_else(
                |_| {
                    ToolchainError::unfixable(
                        "spirv-opt (SPIRV-Tools) is missing and no supported package manager was found",
                        manual_install_hint(),
                    )
                },
                |plan| {
                    ToolchainError::fixable(SpirvToolsInstallation::SystemPackages(plan))
                },
            )
    } else if cfg!(target_os = "windows") {
        // No winget formula ships SPIRV-Tools; the MSYS2 UCRT64 package is the
        // documented install and `doctor --fix` cannot append to PATH anyway.
        ToolchainError::unfixable("spirv-opt (SPIRV-Tools) is missing", manual_install_hint())
    } else {
        ToolchainError::unfixable("spirv-opt (SPIRV-Tools) is missing", manual_install_hint())
    }
}

impl Toolchain for SpirvTools {
    type Installation = SpirvToolsInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        match self.path(host).await {
            Ok(path) => match version_of(host, &path).await {
                Ok(_) => Ok(()),
                // A `spirv-opt` that cannot even report its version would only
                // panic the same way inside `cherenkov-gpu`'s build script.
                Err(error) => Err(ToolchainError::unfixable(
                    format!("`spirv-opt` at {} does not run: {error}", path.display()),
                    manual_install_hint(),
                )),
            },
            Err(_) => Err(missing(host).await),
        }
    }
}

/// Installation plan for `spirv-opt` — the strategy `check` selected.
#[derive(Debug, Clone)]
pub enum SpirvToolsInstallation {
    /// `brew install spirv-tools`.
    Brew,
    /// The host's Linux package manager.
    SystemPackages(LinuxSystemPackagesInstallation),
}

impl SpirvToolsInstallation {
    /// The missing-tool diagnosis and its install command, for the doctor
    /// item's message.
    #[must_use]
    pub fn describe(&self) -> String {
        const WHY: &str = "spirv-opt (SPIRV-Tools) is missing — `cherenkov-gpu` invokes it in its build script for every non-Apple native target (Linux, Windows, Android)";
        match self {
            Self::Brew => format!("{WHY}. `--fix` runs `brew install spirv-tools`."),
            Self::SystemPackages(plan) => {
                format!("{WHY}. `--fix` runs `{}`.", plan.install_command_hint())
            }
        }
    }
}

/// Errors that can occur while installing SPIRV-Tools.
#[derive(Debug, thiserror::Error)]
pub enum FailToInstallSpirvTools {
    /// Homebrew is required for the selected plan but unavailable.
    #[error("Homebrew not found. Please install Homebrew to proceed.")]
    BrewNotFound,
    /// `brew install spirv-tools` failed.
    #[error("Failed to install SPIRV-Tools via Homebrew: {0}")]
    BrewInstall(#[from] CommandError),
    /// The Linux package-manager install failed.
    #[error(transparent)]
    SystemPackages(#[from] FailToInstallLinuxSystemPackages),
}

impl Installation for SpirvToolsInstallation {
    type Error = FailToInstallSpirvTools;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        match self {
            Self::Brew => {
                let brew = Brew::default();
                brew.check(host)
                    .await
                    .map_err(|_| FailToInstallSpirvTools::BrewNotFound)?;
                brew.install(host, "spirv-tools").await?;
                Ok(())
            }
            Self::SystemPackages(plan) => Ok(plan.install(host).await?),
        }
    }
}

#[cfg(test)]
mod host_tests {
    use super::{SpirvTools, SpirvToolsInstallation};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    fn check(machine: &TestMachine) -> Result<(), ToolchainError<SpirvToolsInstallation>> {
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(SpirvTools.check(&host))
    }

    #[test]
    fn ok_when_spirv_opt_on_path() {
        let machine = TestMachine::new();
        machine.install("spirv-opt");
        check(&machine).expect("spirv-opt on PATH must be ok");
    }

    #[test]
    fn version_reports_the_binary_version_line() {
        let machine = TestMachine::new();
        machine.install("spirv-opt");
        let host = machine.host(Vec::<(String, String)>::new());
        assert_eq!(
            smol::block_on(SpirvTools.version(&host)).as_deref(),
            Some("spirv-opt 1.0.0 (waterui-test)"),
        );
    }

    #[test]
    fn missing_is_fixable_when_the_host_can_install_it() {
        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        let result = check(&machine);
        if cfg!(target_os = "windows") {
            // No winget formula ships SPIRV-Tools; the MSYS2 manual step is
            // the documented fix.
            assert!(
                matches!(result, Err(ToolchainError::Unfixable(_))),
                "missing spirv-opt on Windows is a manual MSYS2 install: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "missing spirv-opt with a package manager must be fixable: {result:?}"
            );
        }
    }

    #[test]
    fn missing_without_installer_is_unfixable() {
        let machine = TestMachine::new();
        let result = check(&machine);
        assert!(
            matches!(result, Err(ToolchainError::Unfixable(_))),
            "missing spirv-opt without a package manager must be unfixable: {result:?}"
        );
    }

    #[test]
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    fn install_runs_the_package_manager() {
        use crate::toolchain::Installation;

        let machine = TestMachine::new();
        #[cfg(target_os = "macos")]
        machine.install("brew");
        #[cfg(target_os = "linux")]
        machine.install("apt-get");
        let host = machine.host(Vec::<(String, String)>::new());
        let Err(ToolchainError::Fixable(installation)) = smol::block_on(SpirvTools.check(&host))
        else {
            panic!("missing spirv-opt with a package manager must be fixable");
        };
        smol::block_on(installation.install(&host))
            .expect("installing spirv-tools through the host's package manager must succeed");
    }
}
