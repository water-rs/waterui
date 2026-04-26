use std::net::IpAddr;

use crate::AssetError;
use waterui_url::Url;

#[must_use]
/// Returns whether a path string is an HTTP or HTTPS URL.
pub fn is_remote_url(path: &str) -> bool {
    Url::parse(path)
        .and_then(|parsed| {
            parsed.scheme().map(|scheme| {
                scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
            })
        })
        .unwrap_or(false)
}

/// Rejects non-loopback plain HTTP URLs.
///
/// # Errors
///
/// Returns [`AssetError`] for non-loopback `http://` URLs.
pub fn ensure_http_allowed(url: &str) -> Result<(), AssetError> {
    if has_http_scheme(url) && !is_loopback_http_url(url) {
        return Err(AssetError::http_not_allowed(url));
    }

    Ok(())
}

#[must_use]
/// Returns whether a URL is plain HTTP and targets a loopback host.
pub fn is_loopback_http_url(url: &str) -> bool {
    let Some(parsed) = Url::parse(url) else {
        return false;
    };
    if !parsed
        .scheme()
        .is_some_and(|scheme| scheme.eq_ignore_ascii_case("http"))
    {
        return false;
    }
    parsed
        .host()
        .and_then(normalize_host)
        .is_some_and(is_loopback_host)
}

fn has_http_scheme(url: &str) -> bool {
    Url::parse(url)
        .and_then(|parsed| {
            parsed
                .scheme()
                .map(|scheme| scheme.eq_ignore_ascii_case("http"))
        })
        .unwrap_or(false)
}

fn normalize_host(host: &str) -> Option<&str> {
    if host.is_empty() {
        return None;
    }
    host.strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .or(Some(host))
}

fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_loopback_http_urls() {
        for url in [
            "http://localhost/file.bin",
            "http://LOCALHOST:8080/file.bin",
            "HTTP://localhost/file.bin",
            "http://127.0.0.1/file.bin",
            "http://127.5.6.7:9000/file.bin",
            "http://[::1]/file.bin",
        ] {
            assert!(ensure_http_allowed(url).is_ok(), "expected to allow {url}");
        }
    }

    #[test]
    fn rejects_non_loopback_http_urls() {
        for url in [
            "http://example.com/file.bin",
            "HTTP://example.com/file.bin",
            "http://localhost.evil.com/file.bin",
            "http://127.0.0.1.evil.com/file.bin",
            "http://[::2]/file.bin",
        ] {
            assert!(
                ensure_http_allowed(url).is_err(),
                "expected to reject {url}"
            );
        }
    }
}
