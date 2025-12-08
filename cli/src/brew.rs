use color_eyre::eyre;

use crate::{
    toolchain::{Installation, Toolchain, ToolchainError},
    utils::{run_command, which},
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
    /// Returns an `eyre::Result` indicating success or failure of the installation.
    pub async fn install(&self, name: &str) -> eyre::Result<()> {
        run_command("brew", ["install", name]).await?;
        Ok(())
    }

    /// Install a cask via Homebrew
    ///
    /// # Arguments
    /// * `name` - The name of the cask to install
    /// * `cask` - The cask identifier
    ///
    /// # Errors
    ///
    /// Returns an `eyre::Result` indicating success or failure of the installation.
    pub async fn install_with_cask(&self, name: &str, cask: &str) -> eyre::Result<()> {
        run_command("brew", ["install", "--cask", name, cask]).await?;
        Ok(())
    }
}

impl Toolchain for Brew {
    type Installation = BrewInstallation;
    async fn check(&self) -> Result<(), crate::toolchain::ToolchainError<Self::Installation>> {
        if which("brew").await.is_ok() {
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
    type Error = eyre::Report;

    async fn install(&self) -> Result<(), Self::Error> {
        run_command(
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
