use std::time::Duration;

use zenwave::{Client as _, Method, ResponseExt as _};

use crate::MapLoadError;

pub async fn fetch(
    url: &str,
    limit: usize,
    timeout: Duration,
) -> Result<zenwave::utils::Bytes, MapLoadError> {
    let url = url.to_owned();
    executor_core::spawn(fetch_on_background_executor(url, limit, timeout)).await
}

async fn fetch_on_background_executor(
    url: String,
    limit: usize,
    timeout: Duration,
) -> Result<zenwave::utils::Bytes, MapLoadError> {
    let mut client = zenwave::client().timeout(timeout);
    let response = client
        .method(Method::GET, &url)
        .map_err(|error| MapLoadError::Request {
            url: url.clone(),
            message: error.to_string(),
        })?
        .await
        .map_err(|error| MapLoadError::Request {
            url: url.clone(),
            message: error.to_string(),
        })?;
    let status = response.status();
    if !status.is_success() {
        return Err(MapLoadError::Http {
            url,
            status: status.as_u16(),
        });
    }
    let bytes = response
        .into_bytes_with_limit(limit)
        .await
        .map_err(|error| MapLoadError::ResponseLimit {
            url: url.clone(),
            message: error.to_string(),
        })?;
    Ok(bytes)
}
