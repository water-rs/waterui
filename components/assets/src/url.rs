use std::net::IpAddr;

use crate::AssetError;

#[must_use]
pub fn is_remote_url(path: &str) -> bool {
    strip_http_prefix(path).is_some() || strip_https_prefix(path).is_some()
}

pub fn ensure_http_allowed(url: &str) -> Result<(), AssetError> {
    if has_http_scheme(url) && !is_loopback_http_url(url) {
        return Err(AssetError::http_not_allowed(url));
    }

    Ok(())
}

#[must_use]
pub fn is_loopback_http_url(url: &str) -> bool {
    extract_http_host(url)
        .and_then(normalize_host)
        .is_some_and(is_loopback_host)
}

fn extract_http_host(url: &str) -> Option<&str> {
    let remainder = strip_http_prefix(url)?;
    let authority = remainder.split(['/', '?', '#']).next()?;
    authority.rsplit('@').next()
}

fn has_http_scheme(url: &str) -> bool {
    strip_http_prefix(url).is_some()
}

fn strip_http_prefix(url: &str) -> Option<&str> {
    const HTTP_PREFIX: &str = "http://";
    let prefix_len = HTTP_PREFIX.len();
    let prefix = url.get(..prefix_len)?;
    if prefix.eq_ignore_ascii_case(HTTP_PREFIX) {
        url.get(prefix_len..)
    } else {
        None
    }
}

fn strip_https_prefix(url: &str) -> Option<&str> {
    const HTTPS_PREFIX: &str = "https://";
    let prefix_len = HTTPS_PREFIX.len();
    let prefix = url.get(..prefix_len)?;
    if prefix.eq_ignore_ascii_case(HTTPS_PREFIX) {
        url.get(prefix_len..)
    } else {
        None
    }
}

fn normalize_host(authority: &str) -> Option<&str> {
    if authority.is_empty() {
        return None;
    }

    if let Some(host) = authority.strip_prefix('[') {
        let closing = host.find(']')?;
        return Some(&host[..closing]);
    }

    authority.split(':').next()
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
