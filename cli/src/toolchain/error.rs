//! Toolchain error types.

use std::fmt;

use serde::Serialize;

/// Error returned when a toolchain check fails or cannot be fixed.
#[derive(Debug, Clone)]
pub struct ToolchainError {
    /// The kind of error
    pub kind: ToolchainErrorKind,
    /// Human-readable description of what's missing or wrong
    pub message: String,
    /// Optional suggestion for manual resolution
    pub suggestion: Option<String>,
}

impl ToolchainError {
    /// Create a new error indicating missing components.
    pub fn missing(message: impl Into<String>) -> Self {
        Self {
            kind: ToolchainErrorKind::Missing,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Create a new error indicating the issue cannot be fixed automatically.
    pub fn unfixable(message: impl Into<String>) -> Self {
        Self {
            kind: ToolchainErrorKind::Unfixable,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Create a new error indicating installation failed.
    pub fn install_failed(message: impl Into<String>) -> Self {
        Self {
            kind: ToolchainErrorKind::InstallFailed,
            message: message.into(),
            suggestion: None,
        }
    }

    /// Add a suggestion for manual resolution.
    #[must_use]
    pub fn with_suggestion(mut self, suggestion: impl Into<String>) -> Self {
        self.suggestion = Some(suggestion.into());
        self
    }

    /// Check if this error can potentially be fixed automatically.
    #[must_use]
    pub const fn is_fixable(&self) -> bool {
        matches!(self.kind, ToolchainErrorKind::Missing)
    }
}

impl fmt::Display for ToolchainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(ref suggestion) = self.suggestion {
            write!(f, "\n{suggestion}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ToolchainError {}

/// The kind of toolchain error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolchainErrorKind {
    /// Components are missing but can potentially be installed
    Missing,
    /// The issue cannot be fixed automatically (requires manual intervention)
    Unfixable,
    /// Installation was attempted but failed
    InstallFailed,
}
