//! # `WaterUI` URL Utilities
//!
//! This crate provides ergonomic URL handling for the `WaterUI` framework,
//! supporting both web URLs and local file paths with reactive fetching capabilities.
//!
//! # Compile-Time URLs
//!
//! URLs can be created at compile time using const evaluation:
//!
//! ```
//! use waterui_url::Url;
//!
//! const LOGO: Url = Url::new("https://waterui.dev/logo.png");
//! const STYLESHEET: Url = Url::new("/styles/main.css");
//! ```
//!
//! # Runtime URLs
//!
//! For dynamic URLs, use the `FromStr` trait:
//!
//! ```
//! use waterui_url::Url;
//!
//! let url: Url = "https://example.com".parse()?;
//! # Ok::<(), waterui_url::ParseError>(())
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

mod error;
mod into_url;
mod parser;

use core::str::FromStr;
pub use error::ParseError;
pub use into_url::IntoUrl;

#[cfg(feature = "std")]
use core::error::Error;

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use core::fmt;
use waterui_str::Str;

#[cfg(feature = "std")]
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
#[cfg(feature = "std")]
use sha2::{Digest as _, Sha256};
#[cfg(feature = "std")]
use std::{
    cell::Cell,
    path::{Path, PathBuf},
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};
#[cfg(feature = "std")]
use {
    executor_core::spawn_local,
    nami::Binding,
    nami_core::Signal,
    zenwave::{Client, Method, redirect::FollowRedirect},
};

// ============================================================================
// Parsed Component Types
// ============================================================================

/// Compact byte range representation using u16 indices.
///
/// Special sentinel value `0xFFFF` indicates "not present".
/// This allows representing optional URL components without using `Option<Span>`,
/// saving memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Span {
    start: u16,
    end: u16,
}

impl Span {
    /// Sentinel value indicating the span is not present
    const NONE: Self = Self {
        start: 0xFFFF,
        end: 0xFFFF,
    };

    /// Check if this span represents a present component
    #[inline]
    const fn is_present(self) -> bool {
        self.start != 0xFFFF
    }
}

/// Parsed components for different URL types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum ParsedComponents {
    Web(WebComponents),
    Local(LocalComponents),
    Data(DataComponents),
    Blob(BlobComponents),
    Opaque(OpaqueComponents),
}

/// Components for a scheme that is not followed by `//`, such as `about:blank`,
/// `mailto:me@lexo.cool`, or `javascript:void(0)`.
///
/// Web engines navigate to these routinely, so they are represented rather than
/// mistaken for relative filesystem paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct OpaqueComponents {
    /// URL scheme without the trailing colon (e.g. "about").
    scheme: Span,
    /// Everything after the colon (e.g. "blank"), if any.
    body: Span,
}

/// Components specific to web URLs (http://, https://, etc.).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct WebComponents {
    /// URL scheme (e.g., "https")
    scheme: Span,
    /// Full authority section (user:pass@host:port)
    authority: Span,
    /// Host portion (e.g., "example.com" or "[`::1`]")
    host: Span,
    /// Port number as string (e.g., "8080"), if present
    port: Span,
    /// Path component (e.g., "/api/v1/users")
    path: Span,
    /// Query string without '?' (e.g., "id=123&name=foo")
    query: Span,
    /// Fragment without '#' (e.g., "section")
    fragment: Span,
}

/// Components for local file paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct LocalComponents {
    /// The full path
    path: Span,
    /// Whether this is an absolute path
    is_absolute: bool,
    /// Whether this is a Windows-style path (contains backslashes or drive letter)
    is_windows: bool,
}

/// Components for data URLs (data:...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct DataComponents {
    /// MIME type (e.g., "image/png")
    mime_type: Span,
    /// Encoding (e.g., "base64"), if present
    encoding: Span,
    /// The actual data content
    data: Span,
}

/// Components for blob URLs (blob:...).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct BlobComponents {
    /// The blob identifier
    identifier: Span,
}

/// A URL that can represent either a web URL or a local file path.
///
/// This type provides an ergonomic interface for working with both
/// web URLs (http/https) and local file paths in a unified way.
///
/// # Examples
///
/// ```
/// use waterui_url::Url;
///
/// // Web URLs
/// let web_url = Url::parse("https://example.com/image.jpg").unwrap();
/// assert!(web_url.is_web());
/// assert_eq!(web_url.scheme(), Some("https"));
///
/// // Local file paths
/// # #[cfg(feature = "std")]
/// # {
/// let file_url = Url::from_file_path("/home/user/image.jpg");
/// assert!(file_url.is_local());
/// # }
///
/// // Automatic detection
/// let auto_url = Url::new("./relative/path.png");
/// assert!(auto_url.is_local());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Url {
    /// The original URL string
    inner: Str,
    /// Parsed component offsets (zero-allocation, const-compatible)
    components: ParsedComponents,
}

/// The kind of URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UrlKind {
    /// A web URL (http/https/ftp etc)
    Web,
    /// A local file path (absolute or relative)
    Local,
    /// Data URL (data:)
    Data,
    /// Blob URL (blob:)
    Blob,
}

impl Url {
    /// Creates a URL from a static string at compile time.
    ///
    /// This function can be evaluated at compile time and automatically
    /// detects the URL type (web, local, data, or blob).
    ///
    /// For runtime string parsing, use the `FromStr` trait instead:
    /// `url_string.parse::<Url>()`.
    ///
    /// # Panics
    ///
    /// Panics if the URL is malformed. This enables compile-time syntax checking:
    /// invalid URLs will cause compilation errors when used in const contexts.
    ///
    /// ```compile_fail
    /// # use waterui_url::Url;
    /// // This will fail at compile time - missing host
    /// const INVALID: Url = Url::new("https://");
    /// ```
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// const WEB_URL: Url = Url::new("https://example.com");
    /// const LOCAL_PATH: Url = Url::new("/absolute/path");
    /// const RELATIVE: Url = Url::new("./relative/path");
    /// ```
    #[must_use]
    pub const fn new(url: &'static str) -> Self {
        Self {
            inner: Str::from_static(url),
            components: parser::parse_url(url.as_bytes()),
        }
    }

    /// Parses a URL string, validating it as a proper web URL.
    ///
    /// Returns `None` if the URL is not a valid web URL.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// assert!(Url::parse("https://example.com").is_some());
    /// assert!(Url::parse("http://localhost:3000").is_some());
    /// assert!(Url::parse("/local/path").is_none());
    /// ```
    pub fn parse(url: impl AsRef<str>) -> Option<Self> {
        url.as_ref().parse::<Self>().ok().filter(Self::is_web)
    }

    /// Parses text a person typed, the way an address bar would.
    ///
    /// Anything written with an authority (`https://…`, `file://…`) is taken as
    /// written. A scheme with no authority (`about:blank`, `mailto:…`) is also
    /// taken as written, except when it is really `host:port`. Everything else
    /// gets an `https://` prefix before being parsed again.
    ///
    /// Returns `None` when the input is blank or still does not name a URL.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// assert_eq!(
    ///     Url::parse_user_input("waterui.dev/docs").unwrap().as_str(),
    ///     "https://waterui.dev/docs"
    /// );
    /// assert_eq!(
    ///     Url::parse_user_input("localhost:3000").unwrap().as_str(),
    ///     "https://localhost:3000"
    /// );
    /// assert_eq!(
    ///     Url::parse_user_input("about:blank").unwrap().as_str(),
    ///     "about:blank"
    /// );
    /// assert!(Url::parse_user_input("   ").is_none());
    /// ```
    #[must_use]
    pub fn parse_user_input(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        // Written with an authority: take it exactly as typed, including `file://`.
        if trimmed.contains("://") {
            return trimmed.parse::<Self>().ok();
        }

        // A scheme with no authority is already complete, unless the "scheme" is
        // really a host and the body its port, as in `localhost:3000`.
        if let Ok(url) = trimmed.parse::<Self>()
            && url.is_opaque()
            && !url
                .opaque_body()
                .is_some_and(|body| !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit()))
        {
            return Some(url);
        }

        alloc::format!("https://{trimmed}")
            .parse::<Self>()
            .ok()
            .filter(Self::is_web)
    }

    /// Creates a URL from a file path.
    ///
    /// # Examples
    ///
    /// ```
    /// # #[cfg(feature = "std")]
    /// # {
    /// use waterui_url::Url;
    ///
    /// let url = Url::from_file_path("/home/user/image.jpg");
    /// assert!(url.is_local());
    /// # }
    /// ```
    #[cfg(feature = "std")]
    pub fn from_file_path(path: impl AsRef<Path>) -> Self {
        let path_str = path.as_ref().display().to_string();
        let inner = Str::from(path_str);
        let components = parser::parse_url(inner.as_bytes());
        Self { inner, components }
    }

    /// Creates a URL from a file path string.
    pub fn from_file_path_str(path: impl Into<Str>) -> Self {
        let inner = path.into();
        let components = parser::parse_url(inner.as_bytes());
        Self { inner, components }
    }

    /// Creates a data URL from content and MIME type.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// let url = Url::from_data("image/png", b"...");
    /// assert!(url.is_data());
    /// ```
    #[must_use]
    pub fn from_data(mime_type: &str, data: &[u8]) -> Self {
        use alloc::format;

        let encoded = STANDARD.encode(data);
        let url_str = format!("data:{mime_type};base64,{encoded}");

        let inner = Str::from(url_str);
        let components = parser::parse_url(inner.as_bytes());
        Self { inner, components }
    }

    /// Helper method to extract a string slice from a Span.
    ///
    /// # Safety
    /// The parser ensures that all Span boundaries are valid UTF-8 boundaries.
    #[inline]
    fn slice(&self, span: Span) -> &str {
        if !span.is_present() {
            return "";
        }
        let bytes = self.inner.as_bytes();
        let start = span.start as usize;
        let end = span.end as usize;
        // SAFETY: Parser ensures valid UTF-8 boundaries
        unsafe { core::str::from_utf8_unchecked(&bytes[start..end]) }
    }

    /// Returns true if this is a web URL (http/https/ftp etc).
    #[must_use]
    pub const fn is_web(&self) -> bool {
        matches!(self.components, ParsedComponents::Web(_))
    }

    /// Returns true if this is a local file path.
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self.components, ParsedComponents::Local(_))
    }

    /// Returns true if this is a data URL.
    #[must_use]
    pub const fn is_data(&self) -> bool {
        matches!(self.components, ParsedComponents::Data(_))
    }

    /// Returns true if this is a blob URL.
    #[must_use]
    pub const fn is_blob(&self) -> bool {
        matches!(self.components, ParsedComponents::Blob(_))
    }

    /// Returns true if this URL has a scheme that is not followed by `//`.
    ///
    /// `about:blank`, `mailto:me@lexo.cool` and `javascript:void(0)` are opaque.
    /// These carry no host or path and cannot be resolved against a base URL.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// assert!("about:blank".parse::<Url>().unwrap().is_opaque());
    /// assert!(!"https://waterui.dev".parse::<Url>().unwrap().is_opaque());
    /// ```
    #[must_use]
    pub const fn is_opaque(&self) -> bool {
        matches!(self.components, ParsedComponents::Opaque(_))
    }

    /// Returns true if this is an absolute path or URL.
    #[must_use]
    pub const fn is_absolute(&self) -> bool {
        match self.components {
            ParsedComponents::Web(_)
            | ParsedComponents::Data(_)
            | ParsedComponents::Blob(_)
            | ParsedComponents::Opaque(_) => true,
            ParsedComponents::Local(local) => local.is_absolute,
        }
    }

    /// Returns the inner string representation of the URL.
    #[must_use]
    pub fn inner(&self) -> Str {
        self.inner.clone()
    }

    /// Returns true if this is a relative path.
    #[must_use]
    pub const fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Gets the URL scheme (e.g., "http", "https", "file", "data").
    ///
    /// This is now O(1) - no parsing required!
    #[must_use]
    pub fn scheme(&self) -> Option<&str> {
        match self.components {
            ParsedComponents::Web(web) if web.scheme.is_present() => Some(self.slice(web.scheme)),
            ParsedComponents::Data(_) => Some("data"),
            ParsedComponents::Blob(_) => Some("blob"),
            ParsedComponents::Local(_) => Some("file"),
            ParsedComponents::Opaque(opaque) => Some(self.slice(opaque.scheme)),
            ParsedComponents::Web(_) => None,
        }
    }

    /// Gets the body of an opaque URL — everything after the scheme's colon.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// let url: Url = "mailto:me@lexo.cool".parse().unwrap();
    /// assert_eq!(url.opaque_body(), Some("me@lexo.cool"));
    /// ```
    #[must_use]
    pub fn opaque_body(&self) -> Option<&str> {
        match self.components {
            ParsedComponents::Opaque(opaque) if opaque.body.is_present() => {
                Some(self.slice(opaque.body))
            }
            _ => None,
        }
    }

    /// Gets the host for web URLs.
    ///
    /// This is now O(1) - no parsing required!
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        match self.components {
            ParsedComponents::Web(web) if web.host.is_present() => Some(self.slice(web.host)),
            _ => None,
        }
    }

    /// Gets the path component of the URL.
    ///
    /// This is now O(1) - no parsing required!
    #[must_use]
    pub fn path(&self) -> &str {
        match self.components {
            ParsedComponents::Web(web) if web.path.is_present() => self.slice(web.path),
            ParsedComponents::Web(_) => "/", // No path means root
            ParsedComponents::Local(local) => self.slice(local.path),
            ParsedComponents::Data(_)
            | ParsedComponents::Blob(_)
            | ParsedComponents::Opaque(_) => "",
        }
    }

    /// Gets the port number for web URLs.
    ///
    /// This is a new method enabled by the parsed component structure!
    /// Returns the port as a u16, or None if not present.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match self.components {
            ParsedComponents::Web(web) if web.port.is_present() => {
                self.slice(web.port).parse().ok()
            }
            _ => None,
        }
    }

    /// Gets the query string (without the '?') for web URLs.
    ///
    /// This is a new method enabled by the parsed component structure!
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// const URL: Url = Url::new("https://example.com/path?foo=bar&baz=qux");
    /// assert_eq!(URL.query(), Some("foo=bar&baz=qux"));
    /// ```
    #[must_use]
    pub fn query(&self) -> Option<&str> {
        match self.components {
            ParsedComponents::Web(web) if web.query.is_present() => Some(self.slice(web.query)),
            _ => None,
        }
    }

    /// Gets the fragment (without the '#') for web URLs.
    ///
    /// This is a new method enabled by the parsed component structure!
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// const URL: Url = Url::new("https://example.com/path#section");
    /// assert_eq!(URL.fragment(), Some("section"));
    /// ```
    #[must_use]
    pub fn fragment(&self) -> Option<&str> {
        match self.components {
            ParsedComponents::Web(web) if web.fragment.is_present() => {
                Some(self.slice(web.fragment))
            }
            _ => None,
        }
    }

    /// Gets the authority section (user:pass@host:port) for web URLs.
    ///
    /// This is a new method enabled by the parsed component structure!
    #[must_use]
    pub fn authority(&self) -> Option<&str> {
        match self.components {
            ParsedComponents::Web(web) if web.authority.is_present() => {
                Some(self.slice(web.authority))
            }
            _ => None,
        }
    }

    /// Gets the file extension if present.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let path = self.path();
        let name = path.rsplit('/').next()?;
        let ext_start = name.rfind('.')?;

        if ext_start == 0 || ext_start == name.len() - 1 {
            None
        } else {
            Some(&name[ext_start + 1..])
        }
    }

    /// Gets the filename from the URL path.
    #[must_use]
    pub fn filename(&self) -> Option<&str> {
        let path = self.path();
        path.rsplit('/').next().filter(|s| !s.is_empty())
    }

    /// Joins this URL with a relative path.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// let base = Url::new("https://example.com/images/");
    /// let joined = base.join("photo.jpg");
    /// assert_eq!(joined.as_str(), "https://example.com/images/photo.jpg");
    /// ```
    #[must_use]
    pub fn join(&self, path: &str) -> Self {
        if path.is_empty() {
            return self.clone();
        }

        // If path is absolute, return it as-is
        if matches!(parser::parse_url(path.as_bytes()), ParsedComponents::Web(_))
            || path.starts_with('/')
        {
            return path
                .parse()
                .unwrap_or_else(|_| Self::from_file_path_str(path.to_string()));
        }

        match self.components {
            ParsedComponents::Web(_) => {
                let base = self.inner.as_str();
                let mut result = String::from(base);

                // Ensure base ends with /
                if !result.ends_with('/') {
                    // Check if we have a path after the host
                    if let Some(scheme_end) = result.find("://") {
                        let after_scheme = &result[scheme_end + 3..];
                        if let Some(path_start) = after_scheme.find('/') {
                            // We have a path, check if it looks like a file
                            let full_path_start = scheme_end + 3 + path_start;
                            let after_slash = &result[full_path_start + 1..];
                            if after_slash.contains('.')
                                || after_slash.contains('?')
                                || after_slash.contains('#')
                            {
                                // Remove the file part
                                if let Some(last_slash) = result.rfind('/') {
                                    result.truncate(last_slash + 1);
                                }
                            } else {
                                result.push('/');
                            }
                        } else {
                            // No path after host, add trailing slash
                            result.push('/');
                        }
                    } else {
                        result.push('/');
                    }
                }

                result.push_str(path);
                result
                    .parse()
                    .unwrap_or_else(|_| Self::from_file_path_str(result))
            }
            ParsedComponents::Local(_) => {
                #[cfg(feature = "std")]
                {
                    let base_path = PathBuf::from(self.inner.as_str());
                    let joined = if base_path.is_file() {
                        base_path.parent().unwrap_or(&base_path).join(path)
                    } else {
                        base_path.join(path)
                    };
                    Self::from_file_path(joined)
                }
                #[cfg(not(feature = "std"))]
                {
                    let mut result = String::from(self.inner.as_str());
                    if !result.ends_with('/') && !result.ends_with('\\') {
                        result.push('/');
                    }
                    result.push_str(path);
                    Self::from_file_path_str(result)
                }
            }
            _ => self.clone(),
        }
    }

    /// Fetches the content at this URL.
    ///
    /// Local, blob, and data URLs resolve immediately to themselves. Web URLs
    /// download lazily on first observation and resolve to a cached local file URL.
    /// If a web fetch fails, observing the signal again retries the download
    /// until the fetch instance exhausts its retry budget.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn fetch(&self) -> Fetched {
        Fetched::new(self.clone())
    }

    /// Returns the underlying string representation.
    #[must_use]
    pub const fn as_str(&self) -> &str {
        self.inner.as_str()
    }

    /// Converts this URL to a string.
    #[must_use]
    pub fn into_string(self) -> String {
        String::from(self.inner)
    }

    /// Converts to a file path if this is a local URL.
    #[cfg(feature = "std")]
    #[must_use]
    pub fn to_file_path(&self) -> Option<PathBuf> {
        if self.is_local() {
            Some(PathBuf::from(self.inner.as_str()))
        } else {
            None
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

impl AsRef<str> for Url {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl FromStr for Url {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Runtime parsing must never panic: these strings come from web engines,
        // which navigate to `about:blank`, `data:` documents and `file://` paths
        // as a matter of course.
        match parser::try_parse_url(s.as_bytes()) {
            Ok(components) => Ok(Self {
                inner: Str::from(s.to_string()),
                components,
            }),
            Err(kind) => Err(ParseError::new(kind)),
        }
    }
}

impl From<&'static str> for Url {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Url {
    fn from(value: String) -> Self {
        // Infallible: treat parse failures as local paths
        value
            .as_str()
            .parse()
            .unwrap_or_else(|_| Self::from_file_path_str(value))
    }
}

impl From<Str> for Url {
    fn from(value: Str) -> Self {
        // Infallible: treat parse failures as local paths
        value
            .as_str()
            .parse()
            .unwrap_or_else(|_| Self::from_file_path_str(value))
    }
}

impl<'a> From<Cow<'a, str>> for Url {
    fn from(value: Cow<'a, str>) -> Self {
        match value {
            Cow::Borrowed(s) => s
                .parse()
                .unwrap_or_else(|_| Self::from_file_path_str(s.to_string())),
            Cow::Owned(s) => s.parse().unwrap_or_else(|_| Self::from_file_path_str(s)),
        }
    }
}

impl From<Url> for Str {
    fn from(url: Url) -> Self {
        url.inner
    }
}

// Implement Signal for Url as a constant value
// This allows Url to be used directly with `IntoComputed<Url>`
nami_core::impl_constant!(Url);

#[cfg(feature = "std")]
const FETCH_RETRY_BUDGET: u8 = 3;

#[cfg(feature = "std")]
#[derive(Debug)]
struct FetchedState {
    result: Binding<Option<Url>>,
    in_flight: Cell<bool>,
    remaining_attempts: Cell<u8>,
}

#[cfg(feature = "std")]
impl FetchedState {
    fn new() -> Self {
        Self {
            result: Binding::container(None),
            in_flight: Cell::new(false),
            remaining_attempts: Cell::new(FETCH_RETRY_BUDGET),
        }
    }

    fn try_start(&self) -> bool {
        if self.result.get().is_some() {
            return false;
        }

        if self.in_flight.get() {
            return false;
        }

        let remaining_attempts = self.remaining_attempts.get();
        if remaining_attempts == 0 {
            return false;
        }

        self.remaining_attempts.set(remaining_attempts - 1);
        self.in_flight.set(true);
        true
    }

    fn resolve(&self, fetched: Url) {
        self.result.set(Some(fetched));
        self.in_flight.set(false);
    }

    fn fail(&self) {
        self.in_flight.set(false);
    }

    const fn remaining_attempts(&self) -> u8 {
        self.remaining_attempts.get()
    }
}

/// A reactive signal for fetched URL content.
#[cfg(feature = "std")]
#[derive(Debug, Clone)]
pub struct Fetched {
    url: Url,
    state: Rc<FetchedState>,
}

#[cfg(feature = "std")]
impl Fetched {
    fn new(url: Url) -> Self {
        Self {
            url,
            state: Rc::new(FetchedState::new()),
        }
    }

    fn ensure_started(&self) {
        if !self.state.try_start() {
            return;
        }

        if !self.url.is_web() {
            self.state.resolve(self.url.clone());
            return;
        }

        if let Some(cached) = existing_fetch_cache_url(&self.url) {
            self.state.resolve(cached);
            return;
        }

        let state = self.state.clone();
        let url_string = self.url.as_str().to_owned();
        let path_extension = self.url.extension().map(str::to_owned);
        spawn_local(async move {
            match fetch_remote_to_cache(url_string.clone(), path_extension.clone()).await {
                Ok(fetched) => state.resolve(fetched),
                Err(error) => {
                    state.fail();
                    tracing::warn!(
                        "Url::fetch failed for '{}' ({} retries remaining): {error}",
                        url_string,
                        state.remaining_attempts(),
                    );
                }
            }
        })
        .detach();
    }
}

#[cfg(feature = "std")]
impl Signal for Fetched {
    type Output = Option<Url>;
    type Guard = nami_core::watcher::BoxWatcherGuard;

    fn get(&self) -> Self::Output {
        self.ensure_started();
        self.state.result.get()
    }

    fn watch(
        &self,
        watcher: impl Fn(nami_core::watcher::Context<Self::Output>) + 'static,
    ) -> Self::Guard {
        let guard = self.state.result.watch(watcher);
        self.ensure_started();
        guard
    }
}

#[cfg(feature = "std")]
/// Errors that can occur while downloading a remote resource into memory.
#[derive(Debug)]
pub enum RemoteDownloadError {
    /// The HTTP transport or request pipeline failed.
    Http(Box<zenwave::Error>),
    /// The server returned a non-success status code.
    UnsuccessfulStatus(u16),
    /// Reading the response body failed after a successful response.
    ReadBody(String),
}

#[cfg(feature = "std")]
impl RemoteDownloadError {
    /// Returns the upstream HTTP status code when one exists.
    #[must_use]
    pub const fn status_code(&self) -> Option<u16> {
        match self {
            Self::UnsuccessfulStatus(status) => Some(*status),
            Self::Http(_) | Self::ReadBody(_) => None,
        }
    }
}

#[cfg(feature = "std")]
impl fmt::Display for RemoteDownloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(error) => write!(f, "HTTP transport failed: {error}"),
            Self::UnsuccessfulStatus(status) => {
                write!(f, "upstream returned HTTP status {status}")
            }
            Self::ReadBody(error) => write!(f, "failed to read response body: {error}"),
        }
    }
}

#[cfg(feature = "std")]
impl Error for RemoteDownloadError {}

#[cfg(feature = "std")]
struct DownloadedRemoteBytes {
    bytes: Vec<u8>,
    content_type: Option<String>,
}

#[cfg(feature = "std")]
/// Downloads the bytes for a remote URL without writing them to disk.
///
/// # Errors
///
/// Returns [`RemoteDownloadError`] if the request fails, the server responds
/// with a non-success status, or the body cannot be read.
pub async fn download_remote_bytes(url: &str) -> Result<Vec<u8>, RemoteDownloadError> {
    Ok(download_remote_bytes_with_content_type(url).await?.bytes)
}

#[cfg(feature = "std")]
async fn download_remote_bytes_with_content_type(
    url: &str,
) -> Result<DownloadedRemoteBytes, RemoteDownloadError> {
    let mut client = FollowRedirect::new(zenwave::raw_client());
    let response = client
        .method(Method::GET, url)
        .map_err(|error| RemoteDownloadError::Http(Box::new(error)))?
        .await
        .map_err(|error| RemoteDownloadError::Http(Box::new(error.into())))?;

    if !response.status().is_success() {
        return Err(RemoteDownloadError::UnsuccessfulStatus(
            response.status().as_u16(),
        ));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .map_err(|error| RemoteDownloadError::ReadBody(error.to_string()))?;

    Ok(DownloadedRemoteBytes {
        bytes: bytes.to_vec(),
        content_type,
    })
}

#[cfg(feature = "std")]
#[derive(Debug)]
enum FetchError {
    CacheRootUnavailable,
    CreateCacheDir(std::io::Error),
    Download(Box<RemoteDownloadError>),
    WriteTemp(std::io::Error),
    Persist(std::io::Error),
}

#[cfg(feature = "std")]
impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CacheRootUnavailable => write!(f, "cache directory is unavailable"),
            Self::CreateCacheDir(error) => write!(f, "failed to create cache directory: {error}"),
            Self::Download(error) => fmt::Display::fmt(error, f),
            Self::WriteTemp(error) => write!(f, "failed to write cache temp file: {error}"),
            Self::Persist(error) => write!(f, "failed to persist cached file: {error}"),
        }
    }
}

#[cfg(feature = "std")]
impl Error for FetchError {}

#[cfg(feature = "std")]
fn fetch_cache_root() -> Option<PathBuf> {
    dirs::cache_dir()
        .map(|root| root.join("waterui").join("url-fetch"))
        .or_else(|| {
            let temp_root = std::env::temp_dir();
            (!temp_root.as_os_str().is_empty()).then(|| temp_root.join("waterui").join("url-fetch"))
        })
}

#[cfg(feature = "std")]
fn fetch_cache_key(url: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(url.as_bytes()))
}

#[cfg(feature = "std")]
fn fetch_cache_entry_dir(cache_root: &Path, key: &str) -> PathBuf {
    cache_root.join(key)
}

#[cfg(feature = "std")]
fn cache_payload_path(cache_entry_dir: &Path, extension: Option<&str>) -> PathBuf {
    extension.map_or_else(
        || cache_entry_dir.join("payload"),
        |extension| cache_entry_dir.join(format!("payload.{extension}")),
    )
}

#[cfg(feature = "std")]
fn cache_temp_path(cache_entry_dir: &Path, nonce: u128) -> PathBuf {
    cache_entry_dir.join(format!("incoming-{}-{nonce}", std::process::id()))
}

#[cfg(feature = "std")]
fn existing_fetch_cache_path_in(cache_root: &Path, key: &str) -> Option<PathBuf> {
    let cache_entry_dir = fetch_cache_entry_dir(cache_root, key);
    let payload = cache_entry_dir.join("payload");
    if payload.is_file() {
        return Some(payload);
    }

    let entries = std::fs::read_dir(&cache_entry_dir).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if name
            .strip_prefix("payload.")
            .is_some_and(|extension| !extension.is_empty())
        {
            return Some(path);
        }
    }

    None
}

#[cfg(feature = "std")]
fn existing_fetch_cache_path(url: &Url) -> Option<PathBuf> {
    let cache_root = fetch_cache_root()?;
    let key = fetch_cache_key(url.as_str());
    existing_fetch_cache_path_in(&cache_root, &key)
}

#[cfg(feature = "std")]
fn existing_fetch_cache_url(url: &Url) -> Option<Url> {
    existing_fetch_cache_path(url)
        .map(|path| Url::from_file_path_str(path.to_string_lossy().to_string()))
}

#[cfg(feature = "std")]
fn infer_extension(path_extension: Option<&str>, content_type: Option<&str>) -> Option<String> {
    if let Some(extension) = path_extension {
        return Some(extension.to_ascii_lowercase());
    }

    let content_type = content_type?
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    mime_guess::get_mime_extensions_str(content_type)
        .and_then(|extensions| preferred_extension(extensions))
        .map(str::to_string)
}

#[cfg(feature = "std")]
fn preferred_extension<'a>(extensions: &'a [&'a str]) -> Option<&'a str> {
    [
        "txt", "json", "html", "xml", "css", "js", "jpg", "png", "gif",
    ]
    .into_iter()
    .find(|preferred| extensions.iter().any(|candidate| candidate == preferred))
    .or_else(|| extensions.first().copied())
}

#[cfg(feature = "std")]
async fn fetch_remote_to_cache(
    url: String,
    path_extension: Option<String>,
) -> Result<Url, FetchError> {
    let cache_root = fetch_cache_root().ok_or(FetchError::CacheRootUnavailable)?;
    std::fs::create_dir_all(&cache_root).map_err(FetchError::CreateCacheDir)?;
    let key = fetch_cache_key(&url);

    if let Some(cached) = existing_fetch_cache_path_in(&cache_root, &key) {
        return Ok(Url::from_file_path_str(
            cached.to_string_lossy().to_string(),
        ));
    }

    let cache_entry_dir = fetch_cache_entry_dir(&cache_root, &key);
    std::fs::create_dir_all(&cache_entry_dir).map_err(FetchError::CreateCacheDir)?;

    let downloaded = download_remote_bytes_with_content_type(&url)
        .await
        .map_err(|error| FetchError::Download(Box::new(error)))?;
    let extension = infer_extension(
        path_extension.as_deref(),
        downloaded.content_type.as_deref(),
    );
    let cache_path = cache_payload_path(&cache_entry_dir, extension.as_deref());
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temp_path = cache_temp_path(&cache_entry_dir, nonce);

    std::fs::write(&temp_path, &downloaded.bytes).map_err(FetchError::WriteTemp)?;

    if let Err(error) = std::fs::rename(&temp_path, &cache_path) {
        if cache_path.exists() {
            let _ = std::fs::remove_file(&temp_path);
        } else {
            return Err(FetchError::Persist(error));
        }
    }

    Ok(Url::from_file_path_str(
        cache_path.to_string_lossy().to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "std")]
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn test_const_url_creation() {
        const WEB: Url = Url::new("https://example.com");
        const LOCAL: Url = Url::new("/path/to/file");
        const DATA: Url = Url::new("data:text/plain,hello");
        const BLOB: Url = Url::new("blob:https://example.com/uuid");

        assert!(WEB.is_web());
        assert!(LOCAL.is_local());
        assert!(DATA.is_data());
        assert!(BLOB.is_blob());
    }

    #[test]
    fn test_fromstr_valid_web_urls() {
        let urls = [
            "http://example.com",
            "https://example.com:443/path",
            "ftp://server.com/file",
            "ws://example.com",
            "wss://example.com",
        ];

        for url_str in urls {
            let url: Url = url_str.parse().unwrap();
            assert!(url.is_web(), "Failed for: {url_str}");
        }
    }

    /// Every one of these is a URL a web engine navigates to on its own, and each
    /// used to abort the process: `file://` tripped the "web URL must have a host"
    /// assertion inside the `const` parser, and the rest were reported as invalid
    /// by `Url::parse`, whose caller then panicked.
    #[test]
    fn engine_supplied_urls_parse_instead_of_panicking() {
        let cases = [
            ("about:blank", "about"),
            ("javascript:void(0)", "javascript"),
            ("mailto:me@lexo.cool", "mailto"),
            ("chrome-error://chromewebdata/", "chrome-error"),
            ("file:///tmp/page.html", "file"),
            ("file://localhost/tmp/page.html", "file"),
            ("data:text/plain,hello", "data"),
            ("blob:https://waterui.dev/1234", "blob"),
        ];

        for (raw, scheme) in cases {
            let url: Url = raw
                .parse()
                .unwrap_or_else(|error| panic!("{raw} failed to parse: {error}"));
            assert_eq!(url.scheme(), Some(scheme), "wrong scheme for {raw}");
            assert_eq!(url.as_str(), raw);
        }
    }

    #[test]
    fn file_urls_are_local_paths() {
        let url: Url = "file:///tmp/page.html".parse().unwrap();
        assert!(url.is_local());
        assert!(!url.is_web());
        assert_eq!(url.path(), "/tmp/page.html");

        // The authority is accepted and ignored: both spellings name one file.
        let with_host: Url = "file://localhost/tmp/page.html".parse().unwrap();
        assert_eq!(with_host.path(), "/tmp/page.html");
    }

    #[test]
    fn opaque_urls_expose_their_scheme_and_body() {
        let url: Url = "mailto:me@lexo.cool".parse().unwrap();
        assert!(url.is_opaque());
        assert!(url.is_absolute());
        assert_eq!(url.scheme(), Some("mailto"));
        assert_eq!(url.opaque_body(), Some("me@lexo.cool"));
    }

    /// A single-character "scheme" is a Windows drive letter, so `C:\dir` must not
    /// be reclassified as an opaque `C:` URL.
    #[test]
    fn windows_drive_letters_are_not_opaque_schemes() {
        let url: Url = "C:\\Windows\\file.txt".parse().unwrap();
        assert!(url.is_local());
        assert!(!url.is_opaque());
    }

    #[test]
    fn oversized_and_empty_input_is_reported_not_panicked() {
        assert!("".parse::<Url>().is_err());
        let too_long = alloc::string::String::from_utf8(alloc::vec![b'a'; 70_000]).unwrap();
        assert!(too_long.parse::<Url>().is_err());
    }

    #[test]
    fn parse_user_input_matches_address_bar_expectations() {
        let cases = [
            ("https://waterui.dev", Some("https://waterui.dev")),
            ("waterui.dev/docs", Some("https://waterui.dev/docs")),
            ("  waterui.dev  ", Some("https://waterui.dev")),
            ("localhost:3000", Some("https://localhost:3000")),
            ("about:blank", Some("about:blank")),
            ("file:///tmp/page.html", Some("file:///tmp/page.html")),
            ("mailto:me@lexo.cool", Some("mailto:me@lexo.cool")),
            ("", None),
            ("   ", None),
        ];

        for (input, expected) in cases {
            let parsed = Url::parse_user_input(input);
            assert_eq!(
                parsed.as_ref().map(Url::as_str),
                expected,
                "wrong result for {input:?}"
            );
        }
    }

    #[test]
    fn test_fromstr_local_paths() {
        let paths = [
            "/absolute/path",
            "./relative",
            "file.txt",
            "C:\\Windows\\file.txt",
        ];

        for path in paths {
            let url: Url = path.parse().unwrap();
            assert!(url.is_local(), "Failed for: {path}");
        }
    }

    #[test]
    fn test_fromstr_data_urls() {
        let url: Url = "data:text/plain,hello".parse().unwrap();
        assert!(url.is_data());
    }

    #[test]
    fn test_fromstr_blob_urls() {
        let url: Url = "blob:https://example.com/uuid".parse().unwrap();
        assert!(url.is_blob());
    }

    #[test]
    fn test_fromstr_empty_error() {
        let result: Result<Url, _> = "".parse();
        assert!(result.is_err());
    }

    #[test]
    fn test_web_url_detection() {
        let url = Url::new("https://example.com/image.jpg");
        assert!(url.is_web());
        assert!(!url.is_local());
        assert_eq!(url.scheme(), Some("https"));
        assert_eq!(url.host(), Some("example.com"));
        assert_eq!(url.path(), "/image.jpg");
    }

    #[test]
    fn test_local_path_detection() {
        let url1 = Url::new("/absolute/path/file.txt");
        assert!(url1.is_local());
        assert!(!url1.is_web());
        assert!(url1.is_absolute());

        let url2 = Url::new("./relative/path.txt");
        assert!(url2.is_local());
        assert!(url2.is_relative());

        let url3 = Url::new("file.txt");
        assert!(url3.is_local());
        assert!(url3.is_relative());
    }

    #[test]
    fn test_parse_valid_urls() {
        assert!(Url::parse("http://localhost:3000").is_some());
        assert!(Url::parse("https://example.com/path?query=1").is_some());
        assert!(Url::parse("ftp://server.com/file").is_some());

        assert!(Url::parse("/local/path").is_none());
        assert!(Url::parse("relative/path").is_none());
    }

    #[test]
    fn test_data_url() {
        let url = Url::from_data("image/png", b"test");
        assert!(url.is_data());
        assert!(url.as_str().starts_with("data:image/png;base64,"));
    }

    #[test]
    fn test_extension_extraction() {
        let url1 = Url::new("https://example.com/image.jpg");
        assert_eq!(url1.extension(), Some("jpg"));

        let url2 = Url::new("/path/to/file.tar.gz");
        assert_eq!(url2.extension(), Some("gz"));

        let url3 = Url::new("https://example.com/noext");
        assert_eq!(url3.extension(), None);

        let url4 = Url::new("https://example.com/.hidden");
        assert_eq!(url4.extension(), None);
    }

    #[test]
    fn test_filename_extraction() {
        let url1 = Url::new("https://example.com/path/image.jpg");
        assert_eq!(url1.filename(), Some("image.jpg"));

        let url2 = Url::new("/path/to/file.txt");
        assert_eq!(url2.filename(), Some("file.txt"));

        let url3 = Url::new("https://example.com/");
        assert_eq!(url3.filename(), None);
    }

    #[test]
    fn test_url_joining() {
        let base1 = Url::new("https://example.com/images/");
        let joined1 = base1.join("photo.jpg");
        assert_eq!(joined1.as_str(), "https://example.com/images/photo.jpg");

        let base2 = Url::new("https://example.com/images/old.jpg");
        let joined2 = base2.join("new.jpg");
        assert_eq!(joined2.as_str(), "https://example.com/images/new.jpg");

        let base3 = Url::new("https://example.com");
        let joined3 = base3.join("images/photo.jpg");
        assert_eq!(joined3.as_str(), "https://example.com/images/photo.jpg");
    }

    #[test]
    fn test_windows_paths() {
        let url = Url::new("C:\\Users\\file.txt");
        assert!(url.is_local());
        assert!(url.is_absolute());
    }

    #[test]
    fn test_blob_url() {
        let url = Url::new("blob:https://example.com/uuid");
        assert!(url.is_blob());
        assert_eq!(url.scheme(), Some("blob"));
    }

    #[test]
    fn test_url_host_extraction() {
        let url1 = Url::new("https://example.com/path");
        assert_eq!(url1.host(), Some("example.com"));

        let url2 = Url::new("http://localhost:8080/api");
        assert_eq!(url2.host(), Some("localhost")); // host() now returns only the host, not host:port
        assert_eq!(url2.port(), Some(8080)); // port() is now available!

        let url3 = Url::new("https://sub.domain.com");
        assert_eq!(url3.host(), Some("sub.domain.com"));

        let url4 = Url::new("/local/path");
        assert_eq!(url4.host(), None);
    }

    #[test]
    fn test_complete_url_parsing() {
        // Test a URL with all components
        const FULL_URL: Url =
            Url::new("https://user:pass@example.com:8080/path/to/resource?query=1&foo=bar#section");

        assert_eq!(FULL_URL.scheme(), Some("https"));
        assert_eq!(FULL_URL.host(), Some("example.com"));
        assert_eq!(FULL_URL.port(), Some(8080));
        assert_eq!(FULL_URL.path(), "/path/to/resource");
        assert_eq!(FULL_URL.query(), Some("query=1&foo=bar"));
        assert_eq!(FULL_URL.fragment(), Some("section"));
        assert_eq!(FULL_URL.authority(), Some("user:pass@example.com:8080"));
    }

    #[test]
    fn test_minimal_url() {
        const MIN_URL: Url = Url::new("https://example.com");

        assert_eq!(MIN_URL.scheme(), Some("https"));
        assert_eq!(MIN_URL.host(), Some("example.com"));
        assert_eq!(MIN_URL.port(), None);
        assert_eq!(MIN_URL.path(), "/");
        assert_eq!(MIN_URL.query(), None);
        assert_eq!(MIN_URL.fragment(), None);
    }

    #[test]
    fn test_ipv6_url() {
        const IPV6: Url = Url::new("http://[::1]:8080/test");
        assert_eq!(IPV6.host(), Some("[::1]"));
        assert_eq!(IPV6.port(), Some(8080));
        assert_eq!(IPV6.path(), "/test");
    }

    #[test]
    fn test_query_and_fragment() {
        const URL1: Url = Url::new("https://example.com?foo=bar");
        const URL2: Url = Url::new("https://example.com#section");
        const URL3: Url = Url::new("https://example.com?foo=bar#section");

        assert_eq!(URL1.query(), Some("foo=bar"));
        assert_eq!(URL1.fragment(), None);

        assert_eq!(URL2.query(), None);
        assert_eq!(URL2.fragment(), Some("section"));

        assert_eq!(URL3.query(), Some("foo=bar"));
        assert_eq!(URL3.fragment(), Some("section"));
    }

    #[test]
    fn test_conversions() {
        let url = Url::new("https://example.com");
        let as_str: &str = url.as_ref();
        assert_eq!(as_str, "https://example.com");

        let as_string = url.clone().into_string();
        assert_eq!(as_string, "https://example.com");

        let from_string = Url::from("test".to_string());
        assert_eq!(from_string.as_str(), "test");
    }

    #[test]
    fn test_base64_encoding() {
        let encoded = STANDARD.encode(b"hello");
        assert_eq!(encoded, "aGVsbG8=");

        let encoded2 = STANDARD.encode(b"hi");
        assert_eq!(encoded2, "aGk=");

        let encoded3 = STANDARD.encode(b"test");
        assert_eq!(encoded3, "dGVzdA==");
    }

    #[test]
    fn test_scheme_detection() {
        assert_eq!(Url::new("https://example.com").scheme(), Some("https"));
        assert_eq!(Url::new("http://example.com").scheme(), Some("http"));
        assert_eq!(Url::new("ftp://example.com").scheme(), Some("ftp"));
        assert_eq!(Url::new("ws://example.com").scheme(), Some("ws"));
        assert_eq!(Url::new("waterui://app/settings").scheme(), Some("waterui"));
        assert_eq!(Url::new("waterui://app/settings").host(), Some("app"));
        assert_eq!(Url::new("waterui://app/settings").path(), "/settings");
        assert_eq!(Url::new("data:text/plain,hello").scheme(), Some("data"));
        assert_eq!(
            Url::new("blob:https://example.com/uuid").scheme(),
            Some("blob")
        );
        assert_eq!(Url::new("/local/path").scheme(), Some("file"));
    }

    #[test]
    fn test_path_parsing() {
        let url1 = Url::new("https://example.com/api/v1/users?id=123#section");
        assert_eq!(url1.path(), "/api/v1/users");

        let url2 = Url::new("https://example.com");
        assert_eq!(url2.path(), "/");

        let url3 = Url::new("/local/path/file.txt");
        assert_eq!(url3.path(), "/local/path/file.txt");
    }

    #[test]
    fn test_absolute_relative_detection() {
        assert!(Url::new("https://example.com").is_absolute());
        assert!(Url::new("/absolute/path").is_absolute());
        assert!(Url::new("C:\\Windows\\file.txt").is_absolute());
        assert!(Url::new("data:text/plain,hello").is_absolute());

        assert!(Url::new("relative/path").is_relative());
        assert!(Url::new("./relative/path").is_relative());
        assert!(Url::new("../parent/path").is_relative());
        assert!(Url::new("file.txt").is_relative());
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetch_resolves_local_url_immediately() {
        let url = Url::new("/tmp/example.txt");
        let fetched = url.fetch();
        assert_eq!(fetched.get(), Some(url));
    }

    #[cfg(feature = "std")]
    #[test]
    fn infer_extension_uses_content_type_when_url_has_no_extension() {
        let url = Url::new("https://example.com/download");
        assert_eq!(
            infer_extension(url.extension(), Some("image/png; charset=utf-8")),
            Some(String::from("png"))
        );
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetch_cache_key_has_fixed_length() {
        let long_path = "a".repeat(512);
        let url = Url::from(format!("https://example.com/{long_path}"));
        let key = fetch_cache_key(url.as_str());
        assert_eq!(key.len(), 43);
    }

    #[cfg(feature = "std")]
    #[test]
    fn existing_fetch_cache_path_ignores_incoming_files() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        let temp_dir = std::env::temp_dir().join(format!("waterui-url-cache-test-{unique}"));
        let url = Url::new("https://example.com/download");
        let key = fetch_cache_key(url.as_str());
        let cache_entry_dir = fetch_cache_entry_dir(&temp_dir, &key);
        std::fs::create_dir_all(&cache_entry_dir).expect("cache entry dir should be created");

        let incoming_path = cache_temp_path(&cache_entry_dir, 1);
        std::fs::write(&incoming_path, b"partial").expect("incoming file should be written");
        assert_eq!(existing_fetch_cache_path_in(&temp_dir, &key), None);

        let payload_path = cache_payload_path(&cache_entry_dir, Some("txt"));
        std::fs::write(&payload_path, b"done").expect("payload file should be written");
        assert_eq!(
            existing_fetch_cache_path_in(&temp_dir, &key),
            Some(payload_path)
        );

        std::fs::remove_dir_all(&temp_dir).expect("temp cache dir should be removed");
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetched_state_allows_retry_after_failure() {
        let state = FetchedState::new();
        assert!(state.try_start());
        assert!(!state.try_start());

        state.fail();
        assert!(state.try_start());
        assert_eq!(state.remaining_attempts(), FETCH_RETRY_BUDGET - 2);
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetched_state_stops_restarting_after_resolution() {
        let state = FetchedState::new();
        assert!(state.try_start());

        state.resolve(Url::new("/tmp/example.txt"));
        assert_eq!(state.result.get(), Some(Url::new("/tmp/example.txt")));
        assert!(!state.try_start());
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetched_state_stops_retrying_after_budget_is_exhausted() {
        let state = FetchedState::new();

        for _ in 0..FETCH_RETRY_BUDGET {
            assert!(state.try_start());
            state.fail();
        }

        assert_eq!(state.remaining_attempts(), 0);
        assert!(!state.try_start());
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetch_remote_to_cache_downloads_with_zenwave() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local addr");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("server should accept one connection");
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/plain\r\n",
                "Content-Length: 5\r\n",
                "Connection: close\r\n\r\n",
                "hello"
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });

        let url = Url::from(format!("http://{address}/download"));
        let fetched = futures::executor::block_on(fetch_remote_to_cache(
            url.as_str().to_owned(),
            url.extension().map(str::to_owned),
        ))
        .expect("remote fetch should succeed");
        let fetched_path = fetched
            .to_file_path()
            .expect("remote fetch should resolve to local cache path");
        let contents =
            std::fs::read_to_string(&fetched_path).expect("cached file should be readable");
        assert_eq!(contents, "hello");
        assert_eq!(fetched.extension(), Some("txt"));

        server.join().expect("server thread should finish");
    }

    #[cfg(feature = "std")]
    #[test]
    fn fetch_remote_to_cache_handles_long_urls() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener should bind");
        let address = listener
            .local_addr()
            .expect("listener should have local addr");
        let long_path = "a".repeat(190);
        let expected_request_target = format!("/{long_path}");
        let server = thread::spawn(move || {
            let (mut stream, _) = listener
                .accept()
                .expect("server should accept one connection");
            let mut request = [0u8; 4096];
            let bytes_read = stream
                .read(&mut request)
                .expect("server should read request");
            let request_text =
                core::str::from_utf8(&request[..bytes_read]).expect("request should be utf8");
            let request_line = request_text
                .lines()
                .next()
                .expect("request should contain request line");
            assert!(
                request_line.contains(&expected_request_target),
                "request line should contain long path: {request_line}"
            );
            let response = concat!(
                "HTTP/1.1 200 OK\r\n",
                "Content-Type: text/plain\r\n",
                "Content-Length: 5\r\n",
                "Connection: close\r\n\r\n",
                "hello"
            );
            stream
                .write_all(response.as_bytes())
                .expect("server should write response");
        });

        let url = Url::from(format!("http://{address}/{long_path}"));
        let fetched = futures::executor::block_on(fetch_remote_to_cache(
            url.as_str().to_owned(),
            url.extension().map(str::to_owned),
        ))
        .expect("long-url remote fetch should succeed");
        let fetched_path = fetched
            .to_file_path()
            .expect("long-url fetch should resolve to local cache path");
        let file_name = fetched_path
            .file_name()
            .and_then(|name| name.to_str())
            .expect("fetched file should have a utf8 name");
        assert_eq!(file_name, "payload.txt");
        assert_eq!(
            std::fs::read_to_string(&fetched_path).expect("cached file should be readable"),
            "hello"
        );

        server.join().expect("server thread should finish");
    }
}
