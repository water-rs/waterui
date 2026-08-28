use alloc::{string::ToString, vec::Vec};

use crate::{AssetError, ensure_http_allowed};

/// Downloads bytes from an allowed remote asset URL in the browser.
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
