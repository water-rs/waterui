pub mod apple;

use core::fmt::{Debug, Display};

use color_eyre::eyre;

use crate::{
    doctor::{AnyToolchainIssue, ToolchainIssue},
    project::{self, Project},
};

/// A backend for building and packaging `WaterUI` projects.
///  Implementors should provide methods for initializing, cleaning,
///  and checking requirements for the backend.
pub trait Backend: Display + Debug {
    /// The type of toolchain issues that can be reported by this backend.
    type ToolchainIssue;

    /// Initialize the backend for the given project.
    /// If `dev` is true, initialize in development mode.
    /// This may include setting up debug configurations or
    /// installing development dependencies.
    ///
    /// # Errors
    /// Returns an error if initialization fails.
    fn init(&self, project: &Project, dev: bool) -> eyre::Result<()>;
    /// Check if the backend is already set up for the given project.
    fn is_existing(&self, project: &Project) -> bool;
    /// Clean up any files or configurations added by this backend
    /// for the given project.
    ///  # Errors
    /// Returns an error if cleaning fails.
    fn clean(&self, project: &Project) -> eyre::Result<()>;

    /// Check if the required toolchain components are available
    /// for this backend to function correctly.
    ///  # Errors
    /// Returns a list of toolchain issues if requirements are not met.
    fn check_requirements(&self, project: &Project) -> Result<(), Vec<Self::ToolchainIssue>>;
}

/// A type alias for any backend with dynamic dispatch.
pub type AnyBackend = Box<dyn Backend<ToolchainIssue = AnyToolchainIssue>>;

/// Scan and return a list of available backends.
#[must_use]
pub fn scan_backends(project: &Project) -> Vec<AnyBackend> {
    todo!()
}
