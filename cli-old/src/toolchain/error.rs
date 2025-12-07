//! Toolchain error types.

use color_eyre::eyre;

/// Error returned when a toolchain check fails or cannot be fixed.
#[derive(Debug, thiserror::Error)]
pub enum ToolchainError {
    #[error("Toolchain is missing, run `water doctor` to fix")]
    Fixable { message: String },
    #[error(
        "Toolchain is missing and cannot be automatically fixed: {message}\nSuggestion: {suggestion}"
    )]
    Unfixable { message: String, suggestion: String },
}

pub enum InstallationError {
    FailToInstall { error: eyre::Report },
    UnableToInstall { message: String, suggestion: String },
}

impl ToolchainError {
    pub fn fail(error: impl Into<eyre::Report>) -> Self {
        Self::FailToFix {
            error: error.into(),
        }
    }

    pub fn unfixable(message: impl Into<String>, suggestion: impl Into<String>) -> Self {
        Self::Unfixable {
            message: message.into(),
            suggestion: suggestion.into(),
        }
    }
}
