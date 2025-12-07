//! Composable installation system with parallel execution support.
//!
//! # Example
//!
//! ```ignore
//! use waterui_cli::toolchain::installation::*;
//!
//! let install = Rustup.then(
//!     RustTarget::new("aarch64-linux-android")
//!         .and(RustTarget::new("aarch64-apple-ios"))
//! );
//!
//! // With progress tracking
//! let progress = Progress::new(|name, status| {
//!     println!("{name}: {status:?}");
//! });
//! let report = install.install(progress).await?;
//!
//! // Or without progress tracking
//! let report = install.install(Progress::noop()).await?;
//! ```

pub mod brew;

use std::{
    convert::Infallible,
    fmt::{self, Display},
    future::Future,
};

use serde::Serialize;

use crate::utils::task::Progress;

use super::ToolchainError;

// ============================================================================
// Installation Trait
// ============================================================================

/// A pending installation.
pub trait Installation: Send + Sized {
    /// Execute the installation.
    fn install(self, progress: Progress)
    -> impl Future<Output = Result<(), ToolchainError>> + Send;

    /// Description of what will be installed.
    fn description(&self) -> String;
}

impl Installation for Infallible {
    async fn install(self, progress: Progress) -> Result<(), ToolchainError> {
        unreachable!()
    }
    fn description(&self) -> String {
        unreachable!()
    }
}

// ============================================================================
// Report
// ============================================================================

/// Report of a completed installation.
#[derive(Debug, Clone, Serialize)]
pub struct InstallationReport {
    pub completed: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

impl InstallationReport {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            completed: Vec::new(),
            warnings: Vec::new(),
        }
    }

    pub fn completed(step: impl Into<String>) -> Self {
        Self {
            completed: vec![step.into()],
            warnings: Vec::new(),
        }
    }

    pub fn add_completed(&mut self, step: impl Into<String>) {
        self.completed.push(step.into());
    }

    pub fn add_warning(&mut self, warning: impl Into<String>) {
        self.warnings.push(warning.into());
    }

    pub fn merge(&mut self, other: Self) {
        self.completed.extend(other.completed);
        self.warnings.extend(other.warnings);
    }
}

impl Display for InstallationReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.completed.is_empty() && self.warnings.is_empty() {
            return write!(f, "Nothing installed");
        }
        for step in &self.completed {
            writeln!(f, "  ✓ {step}")?;
        }
        for warning in &self.warnings {
            writeln!(f, "  ⚠ {warning}")?;
        }
        Ok(())
    }
}

impl crate::output::Report for InstallationReport {}
