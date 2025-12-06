pub mod toolchain;

// Re-export ToolchainError as the standard error type for toolchain issues
pub use crate::toolchain::ToolchainError;

/// Type alias for backward compatibility during migration.
///
/// Use `ToolchainError` directly in new code.
pub type AnyToolchainIssue = ToolchainError;
