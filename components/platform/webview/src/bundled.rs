//! Serving a staged directory as an asset origin.
//!
//! [`include_web!`](https://docs.rs/waterui/latest/waterui/macro.include_web.html)
//! expands to a [`WebView::open_assets`](crate::WebView::open_assets) whose
//! server is a [`DirectoryServer`] over the bundle directory the CLI staged —
//! the staged directory is the whole contract: a path that does not resolve to
//! a file inside it is a `404`, and no index or manifest is embedded in the
//! binary.

use std::path::PathBuf;
use std::sync::Arc;

use percent_encoding::percent_decode_str;

use crate::Url;
use crate::assets::{AssetRequest, AssetResponse, AssetServer};

/// The `Content-Security-Policy` a bundled site gets unless `include_web!`'s
/// `csp` argument widens it.
///
/// `default-src 'self'` baseline: same-origin scripts and styles (plus the
/// inline styles a bundler's runtime injects), WebAssembly compilation
/// (`'wasm-unsafe-eval'` permits Wasm only, never `eval`), data/blob images
/// and fonts, same-origin fetches, no plugins, no foreign framing, no
/// base-tag rewriting.
pub const DEFAULT_CSP: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self' data:; connect-src 'self'; media-src 'self' blob:; worker-src 'self' blob:; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

/// Serves one fixed directory as an [`AssetServer`]: the staged web bundle.
///
/// `root` is canonicalized at construction; the serving checks below are what
/// make the staged directory a fixed set — a request path is percent-decoded
/// once, every segment that is empty, `.`, `..`, or contains a NUL or `\` is
/// refused, and the canonicalized answer must stay a file under `root`, so a
/// symlink cannot escape it either. [`crate::assets::dispatch`] has already
/// refused methods other than GET/HEAD and any path that escapes the root by
/// the time this server is consulted.
///
/// With `spa` a miss whose last segment carries no `.` serves `index.html`
/// instead of `404` — client-side routes are files that do not exist; asset
/// misses still answer `404`.
#[derive(Debug, Clone)]
pub struct DirectoryServer {
    root: PathBuf,
    spa: bool,
    // `Str` is not `Send`, and an `AssetServer` must be — the value crosses to
    // the engine's network thread — so it is kept as a `String` and wrapped
    // into the response's header only when a page is served.
    csp: String,
}

impl DirectoryServer {
    /// A server over `root`.
    ///
    /// # Panics
    ///
    /// When `root` is not a directory — the CLI stages the bundle or the build
    /// is wrong; both are build errors, not runtime states.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = root.canonicalize().unwrap_or_else(|error| {
            panic!(
                "asset root '{}' is not a staged directory: {error}",
                root.display()
            )
        });
        assert!(
            root.is_dir(),
            "asset root '{}' is not a staged directory",
            root.display()
        );
        Self {
            root,
            spa: false,
            csp: DEFAULT_CSP.to_string(),
        }
    }

    /// Whether unresolved extensionless paths fall back to `index.html`.
    #[must_use]
    pub const fn spa(mut self, spa: bool) -> Self {
        self.spa = spa;
        self
    }

    /// The `Content-Security-Policy` carried on HTML responses.
    #[must_use]
    pub fn csp(mut self, csp: impl Into<String>) -> Self {
        self.csp = csp.into();
        self
    }

    /// The server as the shared callable every engine's interception calls.
    #[must_use]
    pub fn into_server(self) -> AssetServer {
        Arc::new(self.into_server_fn())
    }

    /// The server as an `impl Fn`, for APIs that take the closure rather than
    /// the shared [`AssetServer`].
    pub fn into_server_fn(self) -> impl Fn(&AssetRequest) -> AssetResponse + Send + Sync + 'static {
        move |request| self.serve(request)
    }

    /// Answers one request; `request.path` is still percent-encoded.
    fn serve(&self, request: &AssetRequest) -> AssetResponse {
        let decoded = percent_decode_str(request.path.as_str()).decode_utf8_lossy();
        let relative = decoded.strip_prefix('/').unwrap_or(&decoded);

        let mut candidate = self.root.clone();
        // A request for "/" names index.html; anything else must name a file.
        if relative.is_empty() {
            candidate.push("index.html");
        } else {
            for segment in relative.split('/') {
                if segment.is_empty()
                    || segment == "."
                    || segment == ".."
                    || segment.contains(['\0', '\\'])
                {
                    return AssetResponse::not_found();
                }
                candidate.push(segment);
            }
        }

        let resolved = candidate
            .canonicalize()
            .ok()
            .filter(|path| path.is_file() && path.starts_with(&self.root));

        let path = if let Some(path) = resolved {
            path
        } else {
            // SPA fallback: a miss whose last segment has no `.` is a
            // client-side route, not an asset.
            let looks_like_route = relative
                .rsplit('/')
                .next()
                .is_some_and(|last| !last.is_empty() && !last.contains('.'));
            if !(self.spa && looks_like_route) {
                return AssetResponse::not_found();
            }
            let Ok(index) = self.root.join("index.html").canonicalize() else {
                return AssetResponse::not_found();
            };
            if index.is_file() {
                index
            } else {
                return AssetResponse::not_found();
            }
        };

        let Ok(body) = std::fs::read(&path) else {
            return AssetResponse::not_found();
        };
        let content_type = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map_or("application/octet-stream", content_type_for);
        let response = AssetResponse::ok(content_type, body);
        if content_type == "text/html" {
            response.with_header("Content-Security-Policy", &self.csp)
        } else {
            response
        }
    }
}

/// The `Content-Type` for a file extension, lowercased.
fn content_type_for(extension: &str) -> &'static str {
    match extension.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" | "map" => "application/json",
        "wasm" => "application/wasm",
        "webmanifest" => "application/manifest+json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "txt" => "text/plain",
        "xml" => "application/xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    }
}

/// The dev-server URL handed to a debug build by `water run`, if any.
///
/// Reads `WATERUI_DEV_URL` from the environment, else a
/// `--waterui-dev-url=<url>` process argument — iOS devices cannot receive
/// environment variables, so the launch channel there is a process argument.
/// The function itself is not gated; callers gate on `debug_assertions`.
///
/// # Panics
///
/// When a present value is not a URL — a malformed handoff is a CLI bug, not
/// a state the app can serve.
#[must_use]
pub fn dev_url() -> Option<Url> {
    dev_url_from(
        std::env::var("WATERUI_DEV_URL").ok().as_deref(),
        std::env::args(),
    )
}

/// The pure core of [`dev_url`]: environment value first, then the
/// `--waterui-dev-url=` process argument.
fn dev_url_from(env: Option<&str>, mut args: impl Iterator<Item = String>) -> Option<Url> {
    if let Some(value) = env {
        return Some(
            value
                .parse()
                .expect("WATERUI_DEV_URL is set but is not a URL"),
        );
    }
    args.find_map(|arg| {
        arg.strip_prefix("--waterui-dev-url=").map(|value| {
            value
                .parse()
                .expect("--waterui-dev-url is present but is not a URL")
        })
    })
}

#[cfg(test)]
mod tests {
    use suiteki::Str;

    use super::*;

    fn stage(files: &[(&str, &[u8])]) -> (tempfile::TempDir, DirectoryServer) {
        let dir = tempfile::tempdir().expect("tempdir");
        for (name, body) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
            std::fs::write(path, body).expect("write fixture");
        }
        let server = DirectoryServer::new(dir.path());
        (dir, server)
    }

    fn get(server: &AssetServer, path: &str) -> AssetResponse {
        server(&AssetRequest {
            method: crate::assets::AssetMethod::Get,
            path: Str::from(path.to_string()),
            query: None,
        })
    }

    #[test]
    fn index_is_served_at_the_root() {
        let (_dir, server) = stage(&[("index.html", b"<html>hi</html>")]);
        let response = get(&server.into_server(), "/");
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"<html>hi</html>");
        assert_eq!(header(&response, "content-type"), "text/html");
        assert_eq!(
            header(&response, "content-security-policy"),
            DEFAULT_CSP,
            "HTML responses carry the CSP"
        );
    }

    #[test]
    fn nested_files_and_mime_table() {
        let (_dir, server) = stage(&[
            ("index.html", b"<html></html>"),
            ("assets/app.js", b"let x = 1;"),
            ("assets/app.wasm", b"\0asm\x01\0\0\0"),
            ("assets/blob.bin", b"\xff\xfe"),
        ]);
        let server = server.into_server();
        let js = get(&server, "/assets/app.js");
        assert_eq!(js.status, 200);
        assert_eq!(header(&js, "content-type"), "text/javascript");
        let wasm = get(&server, "/assets/app.wasm");
        assert_eq!(header(&wasm, "content-type"), "application/wasm");
        let unknown = get(&server, "/assets/blob.bin");
        assert_eq!(header(&unknown, "content-type"), "application/octet-stream");
        assert_eq!(header(&js, "x-content-type-options"), "nosniff");
        assert!(
            header(&js, "content-security-policy").is_empty(),
            "non-HTML responses do not carry the CSP"
        );
    }

    #[test]
    fn missing_files_answer_404() {
        let (_dir, server) = stage(&[("index.html", b"<html></html>")]);
        let server = server.into_server();
        assert_eq!(get(&server, "/nope.js").status, 404);
        assert_eq!(get(&server, "/docs/page").status, 404);
    }

    #[test]
    fn spa_falls_back_for_routes_but_not_assets() {
        let (_dir, server) = stage(&[("index.html", b"<html>spa</html>")]);
        let server = server.spa(true).into_server();
        let route = get(&server, "/docs/getting-started");
        assert_eq!(route.status, 200);
        assert_eq!(route.body, b"<html>spa</html>");
        assert_eq!(get(&server, "/missing.js").status, 404);
        assert_eq!(get(&server, "/docs/deep/route").status, 200);
    }

    #[test]
    fn malformed_segments_answer_404() {
        let (_dir, server) = stage(&[("index.html", b"<html></html>")]);
        let server = server.into_server();
        for path in [
            "/a//b",
            "/./index.html",
            "/../index.html",
            "/a\\b",
            "/%2e%2e/index.html",
            "/%00",
        ] {
            assert_eq!(get(&server, path).status, 404, "{path}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_outside_the_root_answers_404() {
        let outside = tempfile::tempdir().expect("outside tempdir");
        std::fs::write(outside.path().join("secret.txt"), b"secret").expect("write");
        let (dir, server) = stage(&[("index.html", b"<html></html>")]);
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .expect("symlink");
        assert_eq!(get(&server.into_server(), "/link.txt").status, 404);
    }

    #[test]
    fn dev_url_prefers_the_environment() {
        assert_eq!(
            dev_url_from(
                Some("http://localhost:5173/"),
                ["--waterui-dev-url=http://localhost:9999".to_string()].into_iter(),
            )
            .as_ref()
            .map(Url::as_str),
            Some("http://localhost:5173/")
        );
        assert_eq!(
            dev_url_from(
                None,
                [
                    "app".to_string(),
                    "--waterui-dev-url=http://localhost:9999".to_string(),
                ]
                .into_iter(),
            )
            .as_ref()
            .map(Url::as_str),
            Some("http://localhost:9999")
        );
        assert_eq!(dev_url_from(None, ["app".to_string()].into_iter()), None);
    }

    fn header<'a>(response: &'a AssetResponse, name: &str) -> &'a str {
        response
            .headers
            .iter()
            .find(|(key, _)| key.as_str().eq_ignore_ascii_case(name))
            .map_or("", |(_, value)| value.as_str())
    }
}
