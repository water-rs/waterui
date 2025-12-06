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
mod parser;

pub use error::ParseError;

use alloc::borrow::Cow;
use alloc::boxed::Box;

use alloc::string::{String, ToString};
use core::fmt;
use nami_core::Signal;
use waterui_str::Str;

#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

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

        // Base64 encode the data
        let encoded = base64_encode(data);
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

    /// Returns true if this is an absolute path or URL.
    #[must_use]
    pub const fn is_absolute(&self) -> bool {
        match self.components {
            ParsedComponents::Web(_) | ParsedComponents::Data(_) | ParsedComponents::Blob(_) => {
                true
            }
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
            ParsedComponents::Data(_) | ParsedComponents::Blob(_) => "",
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

    /// Fetches the content at this URL (for network resources).
    ///
    /// This returns a reactive signal that can be watched for changes.
    #[must_use]
    pub fn fetch(&self) -> Fetched {
        Fetched { url: self.clone() }
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

impl core::str::FromStr for Url {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseError::empty());
        }

        Ok(Self {
            inner: Str::from(s.to_string()),
            components: parser::parse_url(s.as_bytes()),
        })
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

/// A reactive signal for fetched URL content.
#[derive(Debug, Clone)]
pub struct Fetched {
    url: Url,
}

impl Signal for Fetched {
    type Output = Option<Url>;
    type Guard = nami_core::watcher::BoxWatcherGuard;

    fn get(&self) -> Self::Output {
        // TODO: Implement actual fetching logic
        Some(self.url.clone())
    }

    fn watch(
        &self,
        _watcher: impl Fn(nami_core::watcher::Context<Self::Output>) + 'static,
    ) -> Self::Guard {
        // TODO: Implement actual watching logic
        Box::new(())
    }
}

// Simple base64 encoding for data URLs
fn base64_encode(data: &[u8]) -> String {
    use alloc::vec::Vec;

    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = Vec::with_capacity(data.len().div_ceil(3) * 4);

    for chunk in data.chunks(3) {
        let mut buf = [0u8; 3];
        for (i, &byte) in chunk.iter().enumerate() {
            buf[i] = byte;
        }

        result.push(TABLE[(buf[0] >> 2) as usize]);
        result.push(TABLE[(((buf[0] & 0x03) << 4) | (buf[1] >> 4)) as usize]);

        if chunk.len() > 1 {
            result.push(TABLE[(((buf[1] & 0x0f) << 2) | (buf[2] >> 6)) as usize]);
        } else {
            result.push(b'=');
        }

        if chunk.len() > 2 {
            result.push(TABLE[(buf[2] & 0x3f) as usize]);
        } else {
            result.push(b'=');
        }
    }

    String::from_utf8(result).expect("base64 encoding should produce valid utf8")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(URL1.query(), Some("foo=bar"));
        assert_eq!(URL1.fragment(), None);

        const URL2: Url = Url::new("https://example.com#section");
        assert_eq!(URL2.query(), None);
        assert_eq!(URL2.fragment(), Some("section"));

        const URL3: Url = Url::new("https://example.com?foo=bar#section");
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
        let encoded = base64_encode(b"hello");
        assert_eq!(encoded, "aGVsbG8=");

        let encoded2 = base64_encode(b"hi");
        assert_eq!(encoded2, "aGk=");

        let encoded3 = base64_encode(b"test");
        assert_eq!(encoded3, "dGVzdA==");
    }

    #[test]
    fn test_scheme_detection() {
        assert_eq!(Url::new("https://example.com").scheme(), Some("https"));
        assert_eq!(Url::new("http://example.com").scheme(), Some("http"));
        assert_eq!(Url::new("ftp://example.com").scheme(), Some("ftp"));
        assert_eq!(Url::new("ws://example.com").scheme(), Some("ws"));
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
}
