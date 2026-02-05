//! Large files using memory-mapping.

use alloc::format;
use alloc::string::ToString;
use core::ops::Deref;

#[cfg(feature = "std")]
use std::path::Path;

use crate::AssetError;

/// Large file, memory-mapped for efficient access.
///
/// Use for: ML models (.onnx, .safetensors), large binary data.
///
/// # Warning: Blocking Risks
///
/// Memory-mapped files have inherent blocking risks:
///
/// 1. **Without warming**: Accessing data triggers page faults that **block the current thread**
/// 2. **After warming**: Under memory pressure, the OS may evict warmed pages, causing blocking
///
/// **Recommendation**: Use `LargeFile` on a **background thread**, not the main UI thread.
///
/// # Creating LargeFile
///
/// `LargeFile` **always requires `.await`**, even for local files:
///
/// ```ignore
/// // Local file
/// let model: LargeFile = asset!("model.onnx").await;
///
/// // Remote file (downloaded and cached)
/// let model: LargeFile = asset!("https://huggingface.co/model.onnx").await;
/// ```
///
/// # Usage Pattern
///
/// ```ignore
/// let model: LargeFile = asset!("model.onnx").await;
/// model.warm().await;  // Pre-warm pages (recommended)
///
/// // Process on background thread for safety
/// let result = blocking::unblock(move || {
///     inference_engine.run(&model)
/// }).await;
/// ```
#[cfg(feature = "std")]
pub struct LargeFile {
    /// Memory-mapped data.
    mmap: memmap2::Mmap,
    /// File size in bytes.
    size: usize,
}

#[cfg(feature = "std")]
impl core::fmt::Debug for LargeFile {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("LargeFile")
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "std")]
impl LargeFile {
    /// Create `LargeFile` from a local file path.
    ///
    /// This is async because mmap setup should not block the main thread.
    ///
    /// # Errors
    ///
    /// Returns `AssetError::NotFound` if the file doesn't exist.
    /// Returns `AssetError::Mmap` if memory mapping fails.
    pub async fn from_local(path: impl AsRef<Path> + Send + 'static) -> Result<Self, AssetError> {
        let path = path.as_ref().to_path_buf();

        blocking::unblock(move || {
            let file = std::fs::File::open(&path).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    AssetError::not_found(path.display().to_string())
                } else {
                    AssetError::mmap(path.display().to_string(), e.to_string())
                }
            })?;

            // SAFETY: We ensure the file is not modified while mapped.
            // The file handle is kept alive by the Mmap.
            let mmap = unsafe { memmap2::Mmap::map(&file) }
                .map_err(|e| AssetError::mmap(path.display().to_string(), e.to_string()))?;

            let size = mmap.len();

            Ok(Self { mmap, size })
        })
        .await
    }

    /// Create `LargeFile` from a remote URL.
    ///
    /// Downloads the file to disk cache, then memory-maps it.
    ///
    /// # Errors
    ///
    /// Returns `AssetError::Network` for network errors.
    /// Returns `AssetError::HttpNotAllowed` if using HTTP (not HTTPS) for non-localhost.
    /// Returns `AssetError::Mmap` if memory mapping fails.
    pub async fn from_remote(url: &str) -> Result<Self, AssetError> {
        // Check for HTTP (not allowed except localhost)
        if url.starts_with("http://") && !url.starts_with("http://localhost") {
            return Err(AssetError::http_not_allowed(url));
        }

        // Download to temp file
        let cache_path = download_to_cache(url).await?;

        // Memory map the cached file
        Self::from_local(cache_path).await
    }

    /// File size in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.size
    }

    /// Check if file is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.size == 0
    }

    /// Warm all pages into memory.
    ///
    /// This triggers all page faults on a background thread, ensuring
    /// subsequent access doesn't block.
    ///
    /// # Warning
    ///
    /// Under memory pressure, the OS may evict warmed pages.
    /// For critical paths, prefer using `LargeFile` entirely on background threads.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let model: LargeFile = asset!("model.onnx").await;
    /// model.warm().await;  // Pre-warm
    ///
    /// // Now access is faster (but still may block under memory pressure)
    /// let data: &[u8] = &model;
    /// ```
    pub async fn warm(&self) {
        // Convert to usize for Send-ability across threads.
        // SAFETY: The pointer remains valid because `self` (and thus `self.mmap`)
        // outlives the blocking task - we await the task before returning.
        let ptr_addr = self.mmap.as_ptr() as usize;
        let len = self.mmap.len();

        blocking::unblock(move || {
            const PAGE_SIZE: usize = 4096;
            let mut sum: u8 = 0;

            // Touch every page to trigger page faults on this background thread
            for offset in (0..len).step_by(PAGE_SIZE) {
                // SAFETY: offset is within bounds (0..len), ptr is valid for the
                // duration of this closure because we await the task.
                let ptr = ptr_addr as *const u8;
                sum = sum.wrapping_add(unsafe { *ptr.add(offset) });
            }

            // Prevent dead code elimination
            core::hint::black_box(sum);
        })
        .await;
    }
}

#[cfg(feature = "std")]
impl Deref for LargeFile {
    type Target = [u8];

    /// Dereference to the underlying bytes.
    ///
    /// # Warning
    ///
    /// If `warm()` was not called, this may block due to page faults!
    /// For UI code, always call `warm().await` first, or use on a background thread.
    fn deref(&self) -> &Self::Target {
        &self.mmap
    }
}

#[cfg(feature = "std")]
impl AsRef<[u8]> for LargeFile {
    fn as_ref(&self) -> &[u8] {
        &self.mmap
    }
}

/// Download a remote URL to the cache directory.
#[cfg(feature = "std")]
async fn download_to_cache(url: &str) -> Result<std::path::PathBuf, AssetError> {
    use sha2::{Digest, Sha256};
    use zenwave::{Client, Method, redirect::FollowRedirect};

    // Compute cache path based on URL hash
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let hash = hex::encode(hasher.finalize());

    let cache_dir = dirs::home_dir()
        .ok_or_else(|| AssetError::io("Could not determine home directory"))?
        .join(".water")
        .join("assets");

    std::fs::create_dir_all(&cache_dir)
        .map_err(|e| AssetError::io(format!("Failed to create cache dir: {e}")))?;

    let cache_path = cache_dir.join(&hash);

    // If already cached, return the path
    if cache_path.exists() {
        tracing::debug!("Asset already cached: {url} -> {}", cache_path.display());
        return Ok(cache_path);
    }

    tracing::info!("Downloading asset: {url}");

    // Download
    let mut client = FollowRedirect::new(zenwave::client());
    let response = client
        .method(Method::GET, url)
        .await
        .map_err(|e| AssetError::network(url, None, e.to_string()))?;

    if !response.status().is_success() {
        return Err(AssetError::network(
            url,
            Some(response.status().as_u16()),
            "HTTP request failed",
        ));
    }

    let bytes = response
        .into_body()
        .into_bytes()
        .await
        .map_err(|e| AssetError::network(url, None, e.to_string()))?;

    // Write to cache
    std::fs::write(&cache_path, &bytes)
        .map_err(|e| AssetError::io(format!("Failed to write cache file: {e}")))?;

    tracing::debug!("Cached asset: {url} -> {}", cache_path.display());

    Ok(cache_path)
}

// Stub for no_std (LargeFile requires std)
#[cfg(not(feature = "std"))]
pub struct LargeFile {
    _private: (),
}

#[cfg(not(feature = "std"))]
impl LargeFile {
    /// `LargeFile` requires the `std` feature.
    pub fn from_local(_path: &str) -> Result<Self, AssetError> {
        Err(AssetError::io("LargeFile requires std feature"))
    }
}
