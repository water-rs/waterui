use alloc::{format, string::String, string::ToString, vec::Vec};
use std::{
    io::{ErrorKind, Write},
    path::Path,
};

use crate::{AssetError, ensure_http_allowed};

/// Outcome of an atomic asset write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicWriteOutcome {
    /// Bytes were written to a newly created destination file.
    Written,
    /// Destination already existed and was left untouched.
    ReusedExisting,
}

/// Downloads bytes from an allowed remote asset URL.
///
/// # Errors
///
/// Returns [`AssetError`] when the URL is disallowed or the network request fails.
pub async fn download_remote_bytes(url: &str) -> Result<Vec<u8>, AssetError> {
    ensure_http_allowed(url)?;
    waterui_url::download_remote_bytes(url)
        .await
        .map_err(|error| AssetError::network(url, error.status_code(), error.to_string()))
}

/// Writes bytes to a path using a temporary file plus atomic persist.
///
/// # Errors
///
/// Returns [`AssetError`] when the parent directory cannot be created, the
/// temporary file cannot be written, or the final persist fails.
pub async fn write_bytes_atomically(
    path: &Path,
    bytes: &[u8],
) -> Result<AtomicWriteOutcome, AssetError> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    blocking::unblock(move || write_bytes_atomically_blocking(&path, &bytes)).await
}

fn write_bytes_atomically_blocking(
    path: &Path,
    bytes: &[u8],
) -> Result<AtomicWriteOutcome, AssetError> {
    let parent = path.parent().map_or_else(
        || {
            Err(AssetError::invalid_path(
                path.display().to_string(),
                "missing parent directory",
            ))
        },
        |parent| Ok(parent.to_path_buf()),
    )?;
    std::fs::create_dir_all(&parent).map_err(|error| {
        AssetError::io(format!(
            "Failed to create parent directory {}: {error}",
            parent.display()
        ))
    })?;

    let prefix = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map_or_else(
            || String::from(".waterui.tmp-"),
            |name| format!(".{name}.tmp-"),
        );

    let mut temp_file = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(&parent)
        .map_err(|error| {
            AssetError::io(format!(
                "Failed to create temp file in {}: {error}",
                parent.display()
            ))
        })?;
    temp_file
        .write_all(bytes)
        .map_err(|error| AssetError::io(format!("Failed to write temp file: {error}")))?;
    temp_file
        .flush()
        .map_err(|error| AssetError::io(format!("Failed to flush temp file: {error}")))?;

    match temp_file.persist_noclobber(path) {
        Ok(_) => Ok(AtomicWriteOutcome::Written),
        Err(error) if error.error.kind() == ErrorKind::AlreadyExists => {
            Ok(AtomicWriteOutcome::ReusedExisting)
        }
        Err(error) => Err(AssetError::io(format!(
            "Failed to finalize {}: {}",
            path.display(),
            error.error
        ))),
    }
}
