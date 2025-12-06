//! Rust toolchain and installers.

use std::{
    fmt::{self, Display},
    future::Future,
    process::Command,
};

use tokio::process::Command as AsyncCommand;
use which::which;

use super::{
    ToolchainError, Toolchain,
    installation::{Empty, Installation, InstallationReport, Many, Progress, Sequence},
};

// ============================================================================
// Toolchain
// ============================================================================

/// Rust toolchain configuration.
#[derive(Debug, Clone, Default)]
pub struct Rust {
    targets: Vec<String>,
}

impl Rust {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_target(mut self, target: impl Into<String>) -> Self {
        self.targets.push(target.into());
        self
    }

    #[must_use]
    pub fn with_targets(mut self, targets: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.targets.extend(targets.into_iter().map(Into::into));
        self
    }

    fn has_rustup() -> bool {
        which("rustup").is_ok()
    }

    fn has_rustc() -> bool {
        which("rustc").is_ok()
    }

    async fn installed_targets() -> Vec<String> {
        AsyncCommand::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .await
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }

    async fn missing_targets(&self) -> Vec<String> {
        if self.targets.is_empty() {
            return Vec::new();
        }

        let installed = Self::installed_targets().await;
        self.targets
            .iter()
            .filter(|t| !installed.contains(t))
            .cloned()
            .collect()
    }

    fn missing_targets_sync(&self) -> Vec<String> {
        if self.targets.is_empty() {
            return Vec::new();
        }

        let installed: Vec<String> = Command::new("rustup")
            .args(["target", "list", "--installed"])
            .output()
            .ok()
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        self.targets
            .iter()
            .filter(|t| !installed.contains(t))
            .cloned()
            .collect()
    }
}

/// Installation type for Rust toolchain.
#[derive(Debug)]
pub enum RustInstallation {
    Empty(Empty),
    Targets(Many<RustTarget>),
    Full(Sequence<Rustup, Many<RustTarget>>),
}

impl Display for RustInstallation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(e) => write!(f, "{e}"),
            Self::Targets(t) => write!(f, "{t}"),
            Self::Full(s) => write!(f, "{s}"),
        }
    }
}

impl Installation for RustInstallation {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        match self {
            Self::Empty(_) => "nothing",
            Self::Targets(_) => "Rust targets",
            Self::Full(_) => "Rust toolchain",
        }
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            match self {
                Self::Empty(e) => e.install(progress).await,
                Self::Targets(t) => t.install(progress).await,
                Self::Full(s) => s.install(progress).await,
            }
        }
    }
}

impl Toolchain for Rust {
    type Installation = RustInstallation;

    fn name(&self) -> &'static str {
        "Rust"
    }

    async fn check(&self) -> Result<(), ToolchainError> {
        if !Self::has_rustc() {
            return Err(ToolchainError::missing("Rust is not installed")
                .with_suggestion("Install Rust from https://rustup.rs"));
        }

        let missing = self.missing_targets().await;
        if !missing.is_empty() {
            let targets = missing.join(", ");
            return Err(ToolchainError::missing(format!("Missing Rust targets: {targets}"))
                .with_suggestion(format!("Run: rustup target add {}", missing.join(" "))));
        }

        Ok(())
    }

    fn fix(&self) -> Result<Self::Installation, ToolchainError> {
        if !Self::has_rustup() {
            if Self::has_rustc() {
                return Err(ToolchainError::unfixable("Rust was installed without rustup")
                    .with_suggestion("Install rustup from https://rustup.rs to manage Rust targets"));
            }

            let targets: Vec<_> = self.targets.iter().map(|t| RustTarget::new(t)).collect();
            return Ok(RustInstallation::Full(Sequence {
                first: Rustup,
                second: Many::new(targets),
            }));
        }

        let missing = self.missing_targets_sync();
        if missing.is_empty() {
            return Ok(RustInstallation::Empty(Empty::new()));
        }

        let targets: Vec<_> = missing.into_iter().map(RustTarget::new).collect();
        Ok(RustInstallation::Targets(Many::new(targets)))
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
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        "rustup"
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            progress.start("rustup");
            progress.update("rustup", 0, "downloading installer");

            let status = AsyncCommand::new("sh")
                .args([
                    "-c",
                    "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y",
                ])
                .status()
                .await
                .map_err(|e| {
                    progress.fail("rustup", &format!("{e}"));
                    ToolchainError::install_failed(format!("Failed to run rustup installer: {e}"))
                })?;

            if !status.success() {
                progress.fail("rustup", "installation failed");
                return Err(ToolchainError::install_failed("Rust installation failed")
                    .with_suggestion("Visit https://rustup.rs for manual installation"));
            }

            progress.done("rustup", "installed");
            Ok(InstallationReport::completed("Rust toolchain installed via rustup"))
        }
    }
}

/// Install a single Rust target.
#[derive(Debug, Clone)]
pub struct RustTarget {
    target: String,
}

impl RustTarget {
    pub fn new(target: impl Into<String>) -> Self {
        Self { target: target.into() }
    }
}

impl Display for RustTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "rustup target add {}", self.target)
    }
}

impl Installation for RustTarget {
    type Future = impl Future<Output = Result<InstallationReport, ToolchainError>> + Send;

    fn description(&self) -> &str {
        &self.target
    }

    fn install(self, progress: Progress) -> Self::Future {
        async move {
            progress.start(&self.target);
            progress.update(&self.target, 0, "installing");

            let status = AsyncCommand::new("rustup")
                .args(["target", "add", &self.target])
                .status()
                .await
                .map_err(|e| {
                    progress.fail(&self.target, &format!("{e}"));
                    ToolchainError::install_failed(format!("Failed to run rustup: {e}"))
                })?;

            if !status.success() {
                progress.fail(&self.target, "failed");
                return Err(ToolchainError::install_failed(format!(
                    "rustup target add {} failed",
                    self.target
                ))
                .with_suggestion("Check your internet connection and try again"));
            }

            progress.done(&self.target, "installed");
            Ok(InstallationReport::completed(format!("Installed Rust target: {}", self.target)))
        }
    }
}
