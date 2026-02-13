use std::net::IpAddr;

use crate::AssetError;

pub(crate) fn ensure_http_allowed(url: &str) -> Result<(), AssetError> {
    if url.starts_with("http://") && !is_loopback_http_url(url) {
        return Err(AssetError::http_not_allowed(url));
    }

    Ok(())
}

fn is_loopback_http_url(url: &str) -> bool {
    extract_http_host(url)
        .and_then(normalize_host)
        .is_some_and(is_loopback_host)
}

fn extract_http_host(url: &str) -> Option<&str> {
    let remainder = url.strip_prefix("http://")?;
    let authority = remainder.split(['/', '?', '#']).next()?;
    authority.rsplit('@').next()
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
