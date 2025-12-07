//! Rust toolchain and installers.

use std::fmt::{self, Display};

use color_eyre::eyre::eyre;
use smol::process::Command;
use target_lexicon::Triple;
use which::which;

use crate::utils::task::Progress;

use super::{Toolchain, ToolchainError, installation::Installation};

// ============================================================================
// Toolchain
// ============================================================================

/// Rust toolchain configuration.
#[derive(Debug)]
pub struct Rust {
    targets: Vec<Triple>,
}

impl Rust {
    pub fn new(target: Triple) -> Self {
        Self {
            targets: vec![target],
        }
    }

    pub const fn new_with_targets(targets: Vec<Triple>) -> Self {
        Self { targets }
    }

    fn has_rustup() -> bool {
        which("rustup").is_ok()
    }

    fn has_rustc() -> bool {
        which("rustc").is_ok()
    }

    async fn installed_targets() -> Vec<Triple> {
        let output = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .kill_on_drop(true)
            .output()
            .await
            .expect("failed to execute rustup target list")
            .stdout;

        String::from_utf8(output)
            .expect("invalid utf-8")
            .lines()
            .map(|s| s.parse().expect("invalid target"))
            .collect()
    }

    async fn missing_targets(&self) -> Vec<Triple> {
        let installed = Self::installed_targets().await;

        self.targets
            .iter()
            .cloned()
            .filter(|t| !installed.contains(t))
            .collect()
    }
}

/// Installation type for Rust toolchain.
#[derive(Debug)]
pub struct RustInstallation {
    target: Vec<Triple>,
}

impl Installation for RustInstallation {
    async fn install(self, progress: Progress) -> Result<(), ToolchainError> {
        todo!()
    }

    fn description(&self) -> &str {
        "Rust toolchain installation"
    }
}

impl Toolchain for Rust {
    type Installation = RustInstallation;

    fn name(&self) -> &'static str {
        "Rust"
    }

    async fn check(&self) -> Result<(), ToolchainError> {
        if !Self::has_rustc() {
            return Err(ToolchainError::Fixable {
                message: "Rust is not installed".to_string(),
            });
        }

        if !Self::has_rustup() {
            return Err(ToolchainError::Fixable {
                message: "rustup (Rust toolchain manager) is not installed".to_string(),
            });
        }

        let missing = self.missing_targets().await;
        let missing = missing
            .into_iter()
            .map(|t| t.to_string())
            .collect::<Vec<_>>()
            .join(", ");
        if !missing.is_empty() {
            return Err(ToolchainError::Fixable {
                message: format!("Missing Rust targets: {missing}"),
            });
        }

        Ok(())
    }

    async fn fix(&self) -> Result<Self::Installation, ToolchainError> {
        todo!()
    }
}

// ============================================================================
// Atomic Installers
// ============================================================================

/// Install rustup.
#[derive(Debug, Clone, Copy)]
pub struct Rustup;

impl Display for Rustup {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rustup (Rust toolchain manager)")
    }
}

impl Installation for Rustup {
    fn description(&self) -> String {
        "rustup (Rust toolchain manager)".to_string()
    }

    async fn install(self, progress: Progress) -> Result<(), ToolchainError> {
        progress.start("Installing Rust package manager (rustup)");

        let status = Command::new("sh")
            .args([
                "-c",
                "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
            ])
            .status()
            .await
            .map_err(|e| ToolchainError::fail(eyre!("Failed to run rustup installer: {e}")))?;

        Ok(())
    }
}
