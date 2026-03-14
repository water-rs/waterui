//! Small binary data loaded into memory.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::ops::Deref;

#[cfg(feature = "std")]
use std::path::Path;

use crate::AssetError;
#[cfg(feature = "std")]
use crate::download_remote_bytes;

/// Small binary data, fully loaded into memory.
///
/// Use for: configs, shaders, JSON, small data files.
///
/// # Creating Data
///
/// ```ignore
/// // From local file (sync)
/// let config: Data = asset!("config.json");
///
/// // From remote URL (async)
/// let remote: Data = asset!("https://api.example.com/data.json").await;
///
/// // Embedded at compile time
/// let shader: Data = asset!("shader.wgsl", embed = true);
/// ```
///
/// # Accessing Data
///
/// `Data` implements `Deref<Target = [u8]>`, so it can be used directly as `&[u8]`:
///
/// ```ignore
/// let config: Data = asset!("config.json");
/// let parsed: Config = serde_json::from_slice(&config)?;
/// ```
#[derive(Debug, Clone)]
pub struct Data {
    /// The loaded bytes.
    bytes: Arc<[u8]>,
}

impl Data {
    /// Create `Data` from a byte vector.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self {
            bytes: bytes.into(),
        }
    }

    /// Create `Data` from a static byte slice (for embedded assets).
    #[must_use]
    pub fn from_static(bytes: &'static [u8]) -> Self {
        // Convert static slice to Arc - this is zero-copy for static data
        Self {
            bytes: Arc::from(bytes),
        }
    }

    /// Create `Data` from a local file path (sync).
    ///
    /// # Errors
    ///
    /// Returns `AssetError::NotFound` if the file doesn't exist.
    /// Returns `AssetError::Io` for other I/O errors.
    #[cfg(feature = "std")]
    pub fn from_local(path: impl AsRef<Path>) -> Result<Self, AssetError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AssetError::not_found(path.display().to_string())
            } else {
                AssetError::io(e.to_string())
            }
        })?;
        Ok(Self::from_bytes(bytes))
    }

    /// Create `Data` from a remote URL (async).
    ///
    /// Downloads the content into memory.
    ///
    /// # Errors
    ///
    /// Returns `AssetError::Network` for network errors.
    /// Returns `AssetError::HttpNotAllowed` if using HTTP (not HTTPS) for non-loopback hosts.
    #[cfg(feature = "std")]
    pub async fn from_remote(url: &str) -> Result<Self, AssetError> {
        Ok(Self::from_bytes(download_remote_bytes(url).await?))
    }

    /// Data size in bytes.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Check if data is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Parse data as UTF-8 string.
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid UTF-8.
    pub fn as_str(&self) -> Result<&str, core::str::Utf8Error> {
        core::str::from_utf8(&self.bytes)
    }

    /// Convert to a `String` (UTF-8).
    ///
    /// # Errors
    ///
    /// Returns an error if the data is not valid UTF-8.
    #[cfg(feature = "std")]
    pub fn into_string(self) -> Result<String, core::str::Utf8Error> {
        self.as_str().map(String::from)
    }
}

impl Deref for Data {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.bytes
    }
}

impl AsRef<[u8]> for Data {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl From<Vec<u8>> for Data {
    fn from(bytes: Vec<u8>) -> Self {
        Self::from_bytes(bytes)
    }
}

impl From<&'static [u8]> for Data {
    fn from(bytes: &'static [u8]) -> Self {
        Self::from_static(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn test_from_bytes() {
        let data = Data::from_bytes(vec![1, 2, 3, 4]);
        assert_eq!(data.len(), 4);
        assert_eq!(&*data, &[1, 2, 3, 4]);
    }

    #[test]
    fn test_from_static() {
        static BYTES: &[u8] = b"hello world";
        let data = Data::from_static(BYTES);
        assert_eq!(data.as_str().unwrap(), "hello world");
    }

    #[test]
    fn test_deref() {
        let data = Data::from_bytes(b"test".to_vec());
        let slice: &[u8] = &data;
        assert_eq!(slice, b"test");
    }
}
