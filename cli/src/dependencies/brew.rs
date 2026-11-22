//! Brew toolchain manager for `WaterUI` CLI

use crate::{
    toolchain::{Host, Installation, Toolchain, ToolchainError},
    utils::CommandError,
};

/// Homebrew toolchain manager
#[derive(Debug, Default)]
pub struct Brew {}

impl Brew {
    /// Install a formula via Homebrew
    ///
    /// # Arguments
    /// * `name` - The name of the formula to install
    ///
    /// # Errors
    ///
    /// Returns an error if the `brew install` command fails.
    pub async fn install(&self, host: &Host, name: &str) -> Result<(), CommandError> {
        host.run("brew", ["install", name]).await?;
        Ok(())
    }

    /// Install a cask via Homebrew
    ///
    /// # Arguments
    /// * `cask` - The cask identifier (e.g. "android-studio")
    ///
    /// # Errors
    ///
    /// Returns an error if the `brew install --cask` command fails.
    pub async fn install_cask(&self, host: &Host, cask: &str) -> Result<(), CommandError> {
        host.run("brew", ["install", "--cask", cask]).await?;
        Ok(())
    }
}

impl Toolchain for Brew {
    type Installation = BrewInstallation;
    async fn check(
        &self,
        host: &Host,
    ) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        if host.which("brew").await.is_ok() {
            Ok(())
        } else if cfg!(target_os = "macos") {
            Err(ToolchainError::fixable(BrewInstallation))
        } else {
            Err(ToolchainError::unfixable(
                "Homebrew is only supported on macOS",
                "Why did you try to use Homebrew on a non-macOS system?",
            ))
        }
    }
}

/// Installation procedure for Homebrew
///
/// This will run the official Homebrew installation script.
#[derive(Debug)]
pub struct BrewInstallation;

impl Installation for BrewInstallation {
    type Error = CommandError;

    async fn install(&self, host: &Host) -> Result<(), Self::Error> {
        host.run(
            "sh",
            [
                "-c",
                "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)",
            ],
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Brew;
    use crate::toolchain::testing::TestMachine;
    use crate::toolchain::{Toolchain, ToolchainError};

    #[test]
    fn ok_when_brew_on_path() {
        let machine = TestMachine::new();
        machine.install("brew");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Brew::default().check(&host)).expect("brew on PATH must be ok");
    }

    #[test]
    fn missing_brew_classification_is_platform_scoped() {
        let machine = TestMachine::new();
        let host = machine.host(Vec::<(String, String)>::new());
        let result = smol::block_on(Brew::default().check(&host));
        if cfg!(target_os = "macos") {
            assert!(
                matches!(result, Err(ToolchainError::Fixable(_))),
                "missing brew on macOS must be fixable: {result:?}"
            );
        } else {
            assert!(
                matches!(result, Err(ToolchainError::Unfixable(_))),
                "brew off macOS must be unfixable: {result:?}"
            );
        }
    }

    #[test]
    fn install_runs_brew_install() {
        let machine = TestMachine::new();
        machine.install("brew");
        let host = machine.host(Vec::<(String, String)>::new());
        smol::block_on(Brew::default().install(&host, "cmake"))
            .expect("brew install must succeed against the fake tool");
        smol::block_on(Brew::default().install_cask(&host, "android-studio"))
            .expect("brew install --cask must succeed against the fake tool");
    }
}
