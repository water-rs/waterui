pub mod toolchain;

use core::fmt::{Debug, Display};

use color_eyre::eyre::{self, bail};

/// A toolchain issue that can be reported by a backend.
pub trait ToolchainIssue: Debug + Display + 'static + Send + Sync {
    /// Provide a suggestion to resolve the toolchain issue.
    fn suggestion(&self) -> String {
        "No suggestion available.".to_string()
    }

    /// Attempt to automatically fix the toolchain issue.
    ///
    /// # Errors
    /// Returns an error if the fix process fails.
    fn fix(&self) -> eyre::Result<()> {
        bail!("No automatic fix available.");
    }
}

pub type AnyToolchainIssue = Box<dyn ToolchainIssue>;

impl ToolchainIssue for AnyToolchainIssue {
    fn suggestion(&self) -> String {
        (**self).suggestion()
    }

    fn fix(&self) -> eyre::Result<()> {
        (**self).fix()
    }
}
