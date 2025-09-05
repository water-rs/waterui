//! # `WaterUI` URL Utilities
//!
//! This crate provides ergonomic URL handling for the `WaterUI` framework,
//! supporting both web URLs and local file paths with reactive fetching capabilities.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::borrow::Cow;
use alloc::string::{String, ToString};
use core::fmt;
use nami::Signal;
use waterui_str::Str;

#[cfg(feature = "std")]
use std::path::{Path, PathBuf};

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
    inner: Str,
    kind: UrlKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum UrlKind {
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
    /// Creates a new URL from a string, automatically detecting the type.
    ///
    /// This will try to determine if the input is a web URL or a local path.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_url::Url;
    ///
    /// let url1 = Url::new("https://example.com");
    /// let url2 = Url::new("/absolute/path");
    /// let url3 = Url::new("./relative/path");
    /// ```
    pub fn new(url: impl Into<Str>) -> Self {
        let inner = url.into();
        let kind = Self::detect_kind(&inner);
        Self { inner, kind }
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
        let url_str = url.as_ref();
        
        // Check if it's a valid web URL
        if Self::is_valid_web_url(url_str) {
            Some(Self {
                inner: Str::from(url_str.to_string()),
                kind: UrlKind::Web,
            })
        } else {
            None
        }
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
        Self {
            inner: Str::from(path_str),
            kind: UrlKind::Local,
        }
    }

    /// Creates a URL from a file path string.
    pub fn from_file_path_str(path: impl Into<Str>) -> Self {
        Self {
            inner: path.into(),
            kind: UrlKind::Local,
        }
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
        
        Self {
            inner: Str::from(url_str),
            kind: UrlKind::Data,
        }
    }

    /// Returns true if this is a web URL (http/https/ftp etc).
    #[must_use]
    pub const fn is_web(&self) -> bool {
        matches!(self.kind, UrlKind::Web)
    }

    /// Returns true if this is a local file path.
    #[must_use]
    pub const fn is_local(&self) -> bool {
        matches!(self.kind, UrlKind::Local)
    }

    /// Returns true if this is a data URL.
    #[must_use]
    pub const fn is_data(&self) -> bool {
        matches!(self.kind, UrlKind::Data)
    }

    /// Returns true if this is a blob URL.
    #[must_use]
    pub const fn is_blob(&self) -> bool {
        matches!(self.kind, UrlKind::Blob)
    }

    /// Returns true if this is an absolute path or URL.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        match self.kind {
            UrlKind::Web | UrlKind::Data | UrlKind::Blob => true,
            UrlKind::Local => {
                let s = self.inner.as_str();
                s.starts_with('/') || s.starts_with('\\') || 
                (s.len() >= 3 && s.as_bytes()[1] == b':' && (s.as_bytes()[2] == b'\\' || s.as_bytes()[2] == b'/'))
            }
        }
    }

    /// Returns true if this is a relative path.
    #[must_use]
    pub fn is_relative(&self) -> bool {
        !self.is_absolute()
    }

    /// Gets the URL scheme (e.g., "http", "https", "file", "data").
    #[must_use]
    pub fn scheme(&self) -> Option<&str> {
        match self.kind {
            UrlKind::Web => {
                let s = self.inner.as_str();
                s.find("://").map(|idx| &s[..idx])
            }
            UrlKind::Data => Some("data"),
            UrlKind::Blob => Some("blob"),
            UrlKind::Local => Some("file"),
        }
    }

    /// Gets the host for web URLs.
    #[must_use]
    pub fn host(&self) -> Option<&str> {
        if !self.is_web() {
            return None;
        }

        let s = self.inner.as_str();
        let start = s.find("://").map(|idx| idx + 3)?;
        let end = s[start..].find(&['/', '?', '#'][..]).map_or(s.len(), |idx| start + idx);
        
        Some(&s[start..end])
    }

    /// Gets the path component of the URL.
    #[must_use]
    pub fn path(&self) -> &str {
        match self.kind {
            UrlKind::Local => self.inner.as_str(),
            UrlKind::Web => {
                let s = self.inner.as_str();
                s.find("://").map_or(s, |start_idx| {
                    let after_scheme = &s[start_idx + 3..];
                    after_scheme.find('/').map_or("/", |path_start| {
                        let path = &after_scheme[path_start..];
                        path.find(&['?', '#'][..]).map_or(path, |idx| &path[..idx])
                    })
                })
            }
            UrlKind::Data | UrlKind::Blob => "",
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
        if Self::is_valid_web_url(path) || path.starts_with('/') {
            return Self::new(path.to_string());
        }

        match self.kind {
            UrlKind::Web => {
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
                            if after_slash.contains('.') || after_slash.contains('?') || after_slash.contains('#') {
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
                Self::new(result)
            }
            UrlKind::Local => {
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

    // Helper methods

    #[must_use]
    fn detect_kind(s: &str) -> UrlKind {
        if Self::is_valid_web_url(s) {
            UrlKind::Web
        } else if s.starts_with("data:") {
            UrlKind::Data
        } else if s.starts_with("blob:") {
            UrlKind::Blob
        } else {
            UrlKind::Local
        }
    }

    fn is_valid_web_url(s: &str) -> bool {
        // Common web URL schemes
        const WEB_SCHEMES: &[&str] = &[
            "http://", "https://", "ftp://", "ftps://", 
            "ws://", "wss://", "rtsp://", "rtmp://"
        ];
        
        WEB_SCHEMES.iter().any(|scheme| s.starts_with(scheme))
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

impl From<Str> for Url {
    fn from(value: Str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Url {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&'static str> for Url {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl<'a> From<Cow<'a, str>> for Url {
    fn from(value: Cow<'a, str>) -> Self {
        match value {
            Cow::Borrowed(s) => Self::new(s.to_string()),
            Cow::Owned(s) => Self::new(s),
        }
    }
}

impl From<Url> for Str {
    fn from(url: Url) -> Self {
        url.inner
    }
}

/// A reactive signal for fetched URL content.
#[derive(Debug, Clone)]
pub struct Fetched {
    url: Url,
}

impl Signal for Fetched {
    type Output = Option<Url>;
    type Guard = nami::watcher::BoxWatcherGuard;

    fn get(&self) -> Self::Output {
        // TODO: Implement actual fetching logic
        Some(self.url.clone())
    }

    fn watch(
        &self,
        _watcher: impl Fn(nami::watcher::Context<Self::Output>) + 'static,
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
    
    String::from_utf8(result).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(url2.host(), Some("localhost:8080"));
        
        let url3 = Url::new("https://sub.domain.com");
        assert_eq!(url3.host(), Some("sub.domain.com"));
        
        let url4 = Url::new("/local/path");
        assert_eq!(url4.host(), None);
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
        assert_eq!(Url::new("blob:https://example.com/uuid").scheme(), Some("blob"));
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