//! Error types for asset operations.

use alloc::string::String;
use core::fmt;

/// Errors that can occur during asset operations.
#[derive(Debug)]
pub enum AssetError {
    /// Asset file not found.
    NotFound {
        /// Path or URL that was not found.
        path: String,
    },

    /// I/O error reading asset.
    Io {
        /// Description of the I/O error.
        message: String,
    },

    /// Network error fetching remote asset.
    Network {
        /// URL that failed.
        url: String,
        /// HTTP status code if available.
        status: Option<u16>,
        /// Error message.
        message: String,
    },

    /// Memory mapping failed.
    Mmap {
        /// Path that failed to mmap.
        path: String,
        /// Error message.
        message: String,
    },

    /// Invalid asset path or URL.
    InvalidPath {
        /// The invalid path.
        path: String,
        /// Reason for invalidity.
        reason: String,
    },

    /// HTTP protocol not allowed (must use HTTPS).
    HttpNotAllowed {
        /// The URL that used HTTP.
        url: String,
    },
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { path } => write!(f, "Asset not found: {path}"),
            Self::Io { message } => write!(f, "I/O error: {message}"),
            Self::Network {
                url,
                status,
                message,
            } => {
                if let Some(code) = status {
                    write!(f, "Network error for {url}: HTTP {code} - {message}")
                } else {
                    write!(f, "Network error for {url}: {message}")
                }
            }
            Self::Mmap { path, message } => {
                write!(f, "Memory mapping failed for {path}: {message}")
            }
            Self::InvalidPath { path, reason } => {
                write!(f, "Invalid asset path '{path}': {reason}")
            }
            Self::HttpNotAllowed { url } => {
                write!(
                    f,
                    "HTTP not allowed (use HTTPS): {url}. Only loopback hosts permit HTTP."
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for AssetError {}

impl AssetError {
    /// Create a not found error.
    #[must_use]
    pub fn not_found(path: impl Into<String>) -> Self {
        Self::NotFound { path: path.into() }
    }

    /// Create an I/O error.
    #[must_use]
    pub fn io(message: impl Into<String>) -> Self {
        Self::Io {
            message: message.into(),
        }
    }

    /// Create a network error.
    #[must_use]
    pub fn network(
        url: impl Into<String>,
        status: Option<u16>,
        message: impl Into<String>,
    ) -> Self {
        Self::Network {
            url: url.into(),
            status,
            message: message.into(),
        }
    }

    /// Create an mmap error.
    #[must_use]
    pub fn mmap(path: impl Into<String>, message: impl Into<String>) -> Self {
        Self::Mmap {
            path: path.into(),
            message: message.into(),
        }
    }

    /// Create an invalid path error.
    #[must_use]
    pub fn invalid_path(path: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidPath {
            path: path.into(),
            reason: reason.into(),
        }
    }

    /// Create an HTTP not allowed error.
    #[must_use]
    pub fn http_not_allowed(url: impl Into<String>) -> Self {
        Self::HttpNotAllowed { url: url.into() }
    }
}
