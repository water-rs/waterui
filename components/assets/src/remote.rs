use alloc::{format, string::String, string::ToString, vec::Vec};
use std::{
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use crate::{AssetError, ensure_http_allowed};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomicWriteOutcome {
    Written,
    ReusedExisting,
}

pub async fn download_remote_bytes(url: &str) -> Result<Vec<u8>, AssetError> {
    ensure_http_allowed(url)?;

    use zenwave::{Client, Method, redirect::FollowRedirect};

    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, url)
        .await
        .map_err(|error| AssetError::network(url, None, error.to_string()))?;

    if !response.status().is_success() {
        return Err(AssetError::network(
            url,
            Some(response.status().as_u16()),
            "HTTP request failed",
        ));
    }

    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .map_err(|error| AssetError::network(url, None, error.to_string()))?;

    Ok(bytes.to_vec())
}

pub async fn write_bytes_atomically(
    path: &Path,
    bytes: &[u8],
) -> Result<AtomicWriteOutcome, AssetError> {
    let path = path.to_path_buf();
    let bytes = bytes.to_vec();
    smol::unblock(move || write_bytes_atomically_blocking(path, &bytes)).await
}

fn write_bytes_atomically_blocking(
    path: PathBuf,
    bytes: &[u8],
) -> Result<AtomicWriteOutcome, AssetError> {
    let parent = path.parent().map(Path::to_path_buf).ok_or_else(|| {
        AssetError::invalid_path(path.display().to_string(), "missing parent directory")
    })?;
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
        .map(|name| format!(".{name}.tmp-"))
        .unwrap_or_else(|| String::from(".waterui.tmp-"));

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

    match temp_file.persist_noclobber(&path) {
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
