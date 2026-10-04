//! Toolchain support for `meson`.

use std::path::PathBuf;

use crate::{
    brew::Brew,
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// Toolchain for `meson`.
#[derive(Debug, Clone, Default)]
pub struct Meson;

impl Meson {
    /// Get the path to the `meson` executable.
    ///
    /// # Errors
    /// Returns an error if `meson` is not found in PATH.
    pub async fn path(&self, host: &Host) -> Result<PathBuf, which::Error> {
        host.which("meson").await
    }
}

impl Toolchain for Meson {
    type Installation = MesonInstallation;

    async fn check(&self, host: &Host) -> Result<(), ToolchainError<Self::Installation>> {
        if host.which("meson").await.is_ok() {
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
    /// The Homebrew installation command failed.
    #[error("Failed to install meson via Homebrew: {0}")]
    BrewInstall(#[from] CommandError),
    /// Unsupported platform error.
    #[error(
        "Automatic installation of meson is not supported on this platform. Please install meson manually."
    )]
    UnsupportedPlatform,
}

impl Installation for MesonInstallation {
    type Error = FailToInstallMeson;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        if cfg!(target_os = "macos") {
            let brew = Brew::default();
            brew.check(host)
                .await
                .map_err(|_| FailToInstallMeson::BrewNotFound)?;
            brew.install(host, "meson").await?;
            Ok(())
        } else {
            Err(FailToInstallMeson::UnsupportedPlatform)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Meson, MesonInstallation};
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Installation, Toolchain, ToolchainError};

    #[test]
    fn ok_when_meson_on_path() {
        let machine = TestMachine::new();
        machine.install("meson");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Meson.check(&host)).expect("meson on PATH must be ok");
    }

    #[test]
    fn missing_meson_is_fixable() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Meson.check(&host));
        assert!(
            matches!(result, Err(ToolchainError::Fixable(_))),
            "missing meson must always be fixable: {result:?}"
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn install_runs_brew() {
        let machine = TestMachine::new();
        machine.install("brew");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(MesonInstallation.install(&host))
            .expect("brew install meson must succeed on a host that provides brew");
    }

    #[test]
    #[cfg(not(target_os = "macos"))]
    fn install_fails_off_macos() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(MesonInstallation.install(&host));
        assert!(
            matches!(result, Err(super::FailToInstallMeson::UnsupportedPlatform)),
            "meson install must fail fast outside macOS: {result:?}"
        );
    }
}
