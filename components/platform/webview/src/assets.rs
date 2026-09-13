//! The local asset origin and the serving layer behind it.
//!
//! [`WebView::open_assets`](crate::WebView::open_assets) gives a web view an
//! origin the engine resolves by calling back into Rust instead of the network.
//! Each engine answers its origin through a different facility — a
//! `WKURLSchemeHandler`, an Android `shouldInterceptRequest`, a `WebKit` URI
//! scheme, a CEF `CefSchemeHandlerFactory`, a CDP `Fetch` interception — but the
//! request that reaches the server and the rules that constrain it are the same
//! everywhere, so they live here once: [`dispatch`] is the single entry point
//! every engine calls.

use std::sync::Arc;

use suiteki::Str;

/// The URI scheme WebKit-family engines and CEF serve assets under.
///
/// `WKWebView` cannot register `http`/`https` as custom schemes and `WebKitGTK`,
/// WPE and CEF share the same custom-scheme facility, so those engines produce
/// `waterui://localhost` and mark the scheme secure, local and CORS-enabled.
pub const ASSET_SCHEME: &str = "waterui";

/// The host the `waterui` scheme serves, producing `waterui://localhost`.
pub const ASSET_HOST: &str = "localhost";

/// The reserved host engines that must answer on `https` use instead.
///
/// Android's `WebView` and CDP-intercepted Chromium can only produce a secure
/// context on `https` — Android grants it to `https://`, `http://localhost` and
/// `file://` origins alone — so those engines answer `https://waterui.localhost`.
/// The `.localhost` suffix is RFC 6761: the name never resolves off-box, so a
/// missed interception cannot leak the request to a network.
pub const ASSET_HTTPS_HOST: &str = "waterui.localhost";

/// The origin the WebKit-family engines and CEF answer — `waterui://localhost`,
/// spelled `[ASSET_SCHEME]://[ASSET_HOST]`.
pub const ASSET_ORIGIN: &str = "waterui://localhost";

/// The origin engines that must answer on `https` serve —
/// `https://waterui.localhost`, spelled `https://[ASSET_HTTPS_HOST]`.
pub const ASSET_HTTPS_ORIGIN: &str = "https://waterui.localhost";

/// The only methods an asset origin answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetMethod {
    /// `GET`.
    Get,
    /// `HEAD` — status and headers only; the body is discarded.
    Head,
}

/// One request the engine intercepted on the asset origin.
#[derive(Debug)]
pub struct AssetRequest {
    /// The request method.
    pub method: AssetMethod,
    /// The URL path as received — leading `/`, still percent-encoded. The
    /// server owns decoding and resolution against its fixed asset set.
    pub path: Str,
    /// The query string without `?`, when one was sent.
    pub query: Option<Str>,
}

/// What the asset origin answers.
#[derive(Debug, Clone)]
pub struct AssetResponse {
    /// The HTTP status code.
    pub status: u16,
    /// Response headers as name/value pairs.
    pub headers: Vec<(Str, Str)>,
    /// The response body.
    pub body: Vec<u8>,
}

impl AssetResponse {
    /// A `200` answer carrying `body` as `content_type`.
    ///
    /// `X-Content-Type-Options: nosniff` rides along on every successful
    /// answer so an engine never second-guesses the declared type.
    #[must_use]
    pub fn ok(content_type: &str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            headers: vec![
                (
                    Str::from_static("Content-Type"),
                    Str::from(content_type.to_string()),
                ),
                (
                    Str::from_static("X-Content-Type-Options"),
                    Str::from_static("nosniff"),
                ),
            ],
            body,
        }
    }

    /// `404` with an empty body — a path the fixed asset set does not contain,
    /// and every traversal attempt [`dispatch`] refuses.
    #[must_use]
    pub const fn not_found() -> Self {
        Self {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    /// `405` for anything that is not GET or HEAD.
    #[must_use]
    pub fn method_not_allowed() -> Self {
        Self {
            status: 405,
            headers: vec![(Str::from_static("Allow"), Str::from_static("GET, HEAD"))],
            body: Vec::new(),
        }
    }

    /// Adds a response header.
    #[must_use]
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers
            .push((Str::from(name.to_string()), Str::from(value.to_string())));
        self
    }
}

/// The serving function an application installs.
///
/// Engines call it from whichever thread their network stack uses — a
/// `WKURLSchemeHandler` callback, a `WebViewClient` worker thread, a `WebKit` URI
/// scheme callback, a CEF IO thread — hence `Send + Sync`, and it must not
/// block on the engine's UI thread.
pub type AssetServer = Arc<dyn Fn(&AssetRequest) -> AssetResponse + Send + Sync>;

/// Creation-time configuration for a web view.
///
/// Engines that expose an asset origin register their interception facility
/// while the native view is constructed — `WKWebView` takes scheme handlers on
/// its configuration before the view exists — so this is an input to
/// [`CustomWebViewController::open`](crate::CustomWebViewController::open), not
/// a method that could be called later.
#[derive(Clone, Default)]
pub struct WebViewConfig {
    /// The server behind the engine's local asset origin, when the view was
    /// opened with one.
    pub asset_server: Option<AssetServer>,
}

impl std::fmt::Debug for WebViewConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebViewConfig")
            .field("asset_server", &self.asset_server.as_ref().map(|_| ".."))
            .finish()
    }
}

/// The single entry point every engine routes an intercepted request through.
///
/// Method enforcement and traversal rejection happen here so no engine can
/// accidentally widen the contract: anything but GET or HEAD answers `405`
/// without consulting the server, and a path that escapes the asset root —
/// `..` spelled literally or percent-encoded — answers `404` the same way.
///
/// `path` is the URL path as the engine reports it (leading `/`, still
/// percent-encoded); `query` is the query string without `?`.
pub fn dispatch(
    server: &AssetServer,
    method: &str,
    path: &str,
    query: Option<&str>,
) -> AssetResponse {
    let method = if method.eq_ignore_ascii_case("GET") {
        AssetMethod::Get
    } else if method.eq_ignore_ascii_case("HEAD") {
        AssetMethod::Head
    } else {
        return AssetResponse::method_not_allowed();
    };
    if escapes_root(path) {
        return AssetResponse::not_found();
    }
    let mut response = server(&AssetRequest {
        method,
        path: Str::from(path.to_string()),
        query: query.map(|query| Str::from(query.to_string())),
    });
    if method == AssetMethod::Head {
        // HEAD answers status and headers and nothing else; the body is
        // dropped here so no engine has to remember to.
        response.body = Vec::new();
    }
    response
}

/// Splits `url` into the asset path and query it names under `origin`.
///
/// Engines whose interception API hands over the whole URL — a CEF
/// `CefRequest`, a CDP `Fetch.requestPaused` event — use this to reach the
/// `path`/`query` pair [`dispatch`] takes; engines whose API hands the pieces
/// over separately never need it. `None` means the URL does not name `origin`'s
/// host, which an engine answers `404` rather than serving the wrong tree.
///
/// An explicit port after the host still names the origin. The path comes back
/// as received — leading `/`, still percent-encoded — and the query without
/// `?`; a fragment is dropped, since one never travels to a server.
#[must_use]
pub fn asset_target<'a>(url: &'a str, origin: &str) -> Option<(&'a str, Option<&'a str>)> {
    let rest = url.strip_prefix(origin)?;
    // `origin` ends at the host, so `rest` is the authority tail — an optional
    // port — followed by `path?query#fragment`.
    let rest = if let Some(port) = rest.strip_prefix(':') {
        let start = port.find('/')?;
        &port[start..]
    } else {
        rest
    };
    let rest = rest.split('#').next().unwrap_or_default();
    match rest.split_once('?') {
        Some(("", query)) => Some(("/", Some(query))),
        Some((path, query)) if path.starts_with('/') => Some((path, Some(query))),
        None if rest.is_empty() => Some(("/", None)),
        None if rest.starts_with('/') => Some((rest, None)),
        _ => None,
    }
}

/// Whether `path` — still percent-encoded — resolves outside the asset root.
///
/// The path is decoded once before its segments are checked because an engine
/// hands over the path as received: `/..` and `/%2e%2e` are the same escape,
/// and `%2e%2e%2f`, `..%5c` and friends are only spellings of it.
fn escapes_root(path: &str) -> bool {
    percent_encoding::percent_decode_str(path)
        .decode_utf8_lossy()
        .split(['/', '\\'])
        .any(|segment| segment == "..")
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use super::{
        ASSET_HOST, ASSET_HTTPS_HOST, ASSET_HTTPS_ORIGIN, ASSET_ORIGIN, ASSET_SCHEME,
        AssetRequest, AssetResponse, AssetServer, asset_target, dispatch,
    };

    fn counting_server(calls: &Arc<AtomicUsize>) -> AssetServer {
        let calls = Arc::clone(calls);
        Arc::new(move |request: &AssetRequest| {
            calls.fetch_add(1, Ordering::SeqCst);
            AssetResponse::ok("text/plain", request.path.as_bytes().to_vec())
        })
    }

    #[test]
    fn get_requests_reach_the_server() {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = counting_server(&calls);

        let response = dispatch(&server, "GET", "/index.html", Some("v=1"));

        assert_eq!(response.status, 200);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name.as_str() == "Content-Type" && value.as_str() == "text/plain")
        );
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name.as_str() == "X-Content-Type-Options"
                    && value.as_str() == "nosniff")
        );
    }

    #[test]
    fn head_returns_headers_without_a_body() {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = counting_server(&calls);

        let response = dispatch(&server, "HEAD", "/index.html", None);

        assert_eq!(response.status, 200);
        assert!(response.body.is_empty());
        assert!(!response.headers.is_empty());
    }

    #[test]
    fn anything_but_get_or_head_is_refused_without_consulting_the_server() {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = counting_server(&calls);

        for method in ["POST", "PUT", "DELETE", "OPTIONS", "PATCH"] {
            let response = dispatch(&server, method, "/index.html", None);
            assert_eq!(response.status, 405, "{method} must be refused");
            assert!(
                response
                    .headers
                    .iter()
                    .any(|(name, value)| name.as_str() == "Allow"
                        && value.as_str() == "GET, HEAD")
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn traversal_is_refused_whatever_its_spelling() {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = counting_server(&calls);

        for path in [
            "/../index.html",
            "/%2e%2e/index.html",
            "/%2E%2E/index.html",
            "/..%2findex.html",
            "/%2e%2e%5cindex.html",
            "/a/../../index.html",
            "/%2e%2e",
        ] {
            let response = dispatch(&server, "GET", path, None);
            assert_eq!(response.status, 404, "{path} must not escape");
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn names_that_merely_contain_dots_pass_through() {
        let calls = Arc::new(AtomicUsize::new(0));
        let server = counting_server(&calls);

        for path in ["/app..js", "/dir./file", "/a..b/c"] {
            let response = dispatch(&server, "GET", path, None);
            assert_eq!(response.status, 200, "{path} is a legitimate name");
        }
    }

    #[test]
    fn asset_target_splits_path_and_query() {
        for origin in [ASSET_ORIGIN, ASSET_HTTPS_ORIGIN] {
            assert_eq!(
                asset_target(&format!("{origin}/index.html"), origin),
                Some(("/index.html", None))
            );
            assert_eq!(
                asset_target(&format!("{origin}/a.js?v=1"), origin),
                Some(("/a.js", Some("v=1")))
            );
            assert_eq!(asset_target(origin, origin), Some(("/", None)));
            assert_eq!(asset_target(&format!("{origin}/"), origin), Some(("/", None)));
            // An explicit port still names the asset host.
            assert_eq!(
                asset_target(&format!("{origin}:8443/x"), origin),
                Some(("/x", None))
            );
            // A query or fragment without a path names the root.
            assert_eq!(
                asset_target(&format!("{origin}?v=1"), origin),
                Some(("/", Some("v=1")))
            );
            // A fragment never reaches the server.
            assert_eq!(
                asset_target(&format!("{origin}/app.js#hash"), origin),
                Some(("/app.js", None))
            );
            // The path keeps its percent-encoding.
            assert_eq!(
                asset_target(&format!("{origin}/%2e%2e/x"), origin),
                Some(("/%2e%2e/x", None))
            );
        }
    }

    #[test]
    fn asset_target_refuses_other_hosts() {
        for (origin, url) in [
            (ASSET_ORIGIN, "waterui://other/x"),
            (ASSET_ORIGIN, "https://localhost/x"),
            (ASSET_HTTPS_ORIGIN, "https://waterui.localhost.evil/x"),
            (ASSET_HTTPS_ORIGIN, "waterui://localhost/x"),
        ] {
            assert_eq!(asset_target(url, origin), None, "{url} is not {origin}");
        }
    }

    #[test]
    fn the_reserved_names_are_stable() {
        assert_eq!(ASSET_SCHEME, "waterui");
        assert_eq!(ASSET_HOST, "localhost");
        assert_eq!(ASSET_HTTPS_HOST, "waterui.localhost");
        assert_eq!(ASSET_ORIGIN, "waterui://localhost");
        assert_eq!(ASSET_HTTPS_ORIGIN, "https://waterui.localhost");
    }
}
