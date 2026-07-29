//! Shared build-script helpers for `WaterUI` code generation.

pub mod icons;
pub use shaderloom::build as shader;

use std::{fs, path::Path};

use heck::{ToShoutySnakeCase, ToSnakeCase};

/// Returns whether the source points at an HTTP(S) URL.
#[must_use]
pub fn is_http_url(source: &str) -> bool {
    has_prefix_ignore_ascii_case(source, "http://")
        || has_prefix_ignore_ascii_case(source, "https://")
}

/// Loads text from either a local file or a cached HTTP(S) source.
///
/// # Errors
///
/// Returns an error if the cached file, local file, or HTTP download cannot be read.
pub fn load_cached_text(
    source: &str,
    cache_path: &Path,
    description: &str,
) -> Result<String, String> {
    if is_http_url(source) {
        if cache_path.exists() {
            return fs::read_to_string(cache_path)
                .map_err(|error| format!("Failed to read cached {description}: {error}"));
        }
        let content = fetch_text(source, description)?;
        let _ = fs::write(cache_path, &content);
        Ok(content)
    } else {
        fs::read_to_string(source)
            .map_err(|error| format!("Failed to read {description} from {source}: {error}"))
    }
}

/// Fetches text from either a local file or an HTTP(S) source.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or the local file cannot be read.
pub fn fetch_text(source: &str, description: &str) -> Result<String, String> {
    if is_http_url(source) {
        tracing::debug!(description, source, "Downloading build input");
        ureq::get(source)
            .call()
            .map_err(|error| format!("Download failed: {error}"))?
            .into_body()
            .read_to_string()
            .map_err(|error| format!("Failed to read response: {error}"))
    } else {
        tracing::debug!(description, source, "Reading local build input");
        fs::read_to_string(source).map_err(|error| format!("Failed to read file: {error}"))
    }
}

/// Converts a source name into a Rust constant identifier.
#[must_use]
pub fn rust_const_name(name: &str) -> String {
    let shouty = name.to_shouty_snake_case();
    if shouty.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        format!("ICON_{shouty}")
    } else {
        shouty
    }
}

/// Converts a source name into a Rust function identifier.
#[must_use]
pub fn rust_fn_name(name: &str) -> String {
    let snake = name.to_snake_case();
    let candidate = if snake.chars().next().is_some_and(|ch| ch.is_ascii_digit())
        || matches!(snake.as_str(), "crate" | "self" | "super")
    {
        format!("icon_{snake}")
    } else {
        snake
    };

    if matches!(candidate.as_str(), "crate" | "self" | "super") {
        format!("icon_{candidate}")
    } else {
        format!("r#{candidate}")
    }
}

fn has_prefix_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|candidate| candidate.eq_ignore_ascii_case(prefix))
}
