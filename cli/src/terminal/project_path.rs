use std::path::{Path, PathBuf};

use color_eyre::eyre::{Result, WrapErr};

pub fn canonicalize(path: &Path) -> Result<PathBuf> {
    dunce::canonicalize(path)
        .wrap_err_with(|| format!("Failed to canonicalize path: {}", path.display()))
}
