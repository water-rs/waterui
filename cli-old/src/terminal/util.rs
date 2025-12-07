use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, bail};
use which::which;

/// Return the workspace root for the CLI package.
///
/// # Panics
/// Panics if the CLI manifest directory does not have a parent.
#[must_use]
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("CLI manifest to have parent")
        .to_path_buf()
}

/// Ensure a directory exists, creating missing parents when needed.
///
/// # Errors
/// Returns an error if the directory cannot be created.
pub fn ensure_directory(path: &Path) -> Result<()> {
    if !path.exists() {
        std::fs::create_dir_all(path)?;
    }
    Ok(())
}

/// Check whether a tool exists on the PATH and produce a helpful error otherwise.
///
/// # Errors
/// Returns an error if the tool cannot be located.
pub fn require_tool(tool: &str, hint: &str) -> Result<()> {
    if which(tool).is_ok() {
        Ok(())
    } else {
        bail!("{tool} not found. {hint}")
    }
}
