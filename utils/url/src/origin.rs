//! The origin a URL names, as web engines compare it.

use core::fmt;
use suiteki::Str;

use crate::Url;

/// A tuple origin: scheme, host and port, with the scheme's default port
/// elided.
///
/// This is the WHATWG URL Standard's tuple origin with the parser's
/// normalization applied — scheme and host lowercased, a port equal to the
/// scheme's default (`http`/`ws` 80, `https`/`wss` 443, `ftp` 21) dropped — so
/// `https://A.dev:443/x` and `https://a.dev/y` have one origin, and its
/// [`Display`](fmt::Display) form is the ASCII serialization engines report
/// for a frame (`WKSecurityOrigin`, a CDP execution context, `WebKitFrame`).
///
/// Two departures from the standard follow how engines treat the URLs a web
/// view actually loads. A hierarchical custom scheme with a host —
/// `waterui://localhost`, served by a registered scheme handler — has a tuple
/// origin, as `WebKit` gives it one, where the standard would make it opaque.
/// A `blob:` URL is opaque here rather than inheriting its creator's origin,
/// since nothing in the URL itself authenticates that creator.
///
/// An opaque origin — a local path, `data:`, `blob:`, `about:` and every
/// other URL without a hierarchical host — has no value:
/// [`Url::origin`] returns `None`, and the standard's rule that an opaque
/// origin equals nothing but itself means callers never compare two `None`s
/// as one origin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    scheme: Str,
    host: Str,
    port: Option<u16>,
}

impl Origin {
    /// The scheme, lowercased.
    #[must_use]
    pub fn scheme(&self) -> &str {
        &self.scheme
    }

    /// The host, lowercased.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The port, or `None` when the URL named none or named the scheme's
    /// default.
    #[must_use]
    pub const fn port(&self) -> Option<u16> {
        self.port
    }
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}://{}", self.scheme, self.host)?;
        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }
        Ok(())
    }
}

/// The port a scheme uses when a URL names none, for the schemes the URL
/// Standard defines one.
fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "http" | "ws" => Some(80),
        "https" | "wss" => Some(443),
        "ftp" => Some(21),
        _ => None,
    }
}

impl Url {
    /// The origin this URL names, or `None` when it is opaque.
    ///
    /// See [`Origin`] for the definition.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// let at = |url: &str| url.parse::<Url>().unwrap();
    /// assert_eq!(
    ///     at("https://a.dev:443/x").origin(),
    ///     at("https://A.dev/y").origin(),
    /// );
    /// assert_eq!(
    ///     at("http://localhost:3000/").origin().unwrap().to_string(),
    ///     "http://localhost:3000",
    /// );
    /// assert_eq!(at("about:blank").origin(), None);
    /// ```
    #[must_use]
    pub fn origin(&self) -> Option<Origin> {
        if !self.is_web() {
            return None;
        }
        let scheme = Str::from(self.scheme()?.to_ascii_lowercase());
        let host = Str::from(self.host()?.to_ascii_lowercase());
        let port = self
            .port()
            .filter(|&port| default_port(&scheme) != Some(port));
        Some(Origin { scheme, host, port })
    }
}

#[cfg(test)]
mod tests {
    use crate::Url;
    use alloc::string::ToString;

    fn at(url: &str) -> Url {
        url.parse().expect("a valid URL")
    }

    #[test]
    fn a_default_port_is_the_origin_without_one() {
        for (with, without) in [
            ("https://a.dev:443/x", "https://a.dev/"),
            ("http://a.dev:80", "http://a.dev/y?q"),
            ("wss://a.dev:443", "wss://a.dev"),
            ("ftp://a.dev:21", "ftp://a.dev"),
        ] {
            assert_eq!(at(with).origin(), at(without).origin(), "{with}");
            assert_eq!(at(with).origin().expect("a tuple origin").port(), None);
        }
    }

    #[test]
    fn scheme_host_and_a_non_default_port_distinguish_origins() {
        let base = at("https://a.dev/").origin();
        for other in [
            "http://a.dev/",
            "https://b.dev/",
            "https://a.dev:8443/",
            // 80 is only `http`'s default.
            "https://a.dev:80/",
        ] {
            assert_ne!(base, at(other).origin(), "{other}");
        }
    }

    #[test]
    fn scheme_and_host_compare_without_case() {
        assert_eq!(
            at("HTTPS://App.Dev/").origin(),
            at("https://app.dev").origin()
        );
    }

    #[test]
    fn the_display_form_is_the_serialization_engines_report() {
        for (url, origin) in [
            ("https://a.dev:443/x", "https://a.dev"),
            ("http://localhost:3000/app", "http://localhost:3000"),
            ("waterui://localhost/index.html", "waterui://localhost"),
            ("http://[::1]:8080/", "http://[::1]:8080"),
        ] {
            assert_eq!(
                at(url).origin().expect("a tuple origin").to_string(),
                origin,
                "{url}"
            );
        }
    }

    #[test]
    fn urls_without_a_hierarchical_host_are_opaque() {
        for url in [
            "about:blank",
            "data:text/html,hi",
            "blob:https://a.dev/1",
            "mailto:me@lexo.cool",
            "file:///tmp/page.html",
            "/tmp/page.html",
        ] {
            assert_eq!(at(url).origin(), None, "{url}");
        }
    }
}
