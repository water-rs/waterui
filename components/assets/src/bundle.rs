use alloc::borrow::Cow;
use alloc::string::ToString;
use alloc::vec::Vec;

use std::env;
use std::path::PathBuf;

use waterui_core::{Environment, View};
use waterui_media::Photo;
use waterui_url::Url;
use waterui_video::{Video, VideoPlayer};

use crate::{AssetError, Data, LargeFile};

const ASSETS_ENV: &str = "WATERUI_ASSETS_ROOT";

/// Asset bundle rooted at the packaged `WaterUI` assets directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bundle {
    prefix: &'static str,
}

impl Bundle {
    /// Returns the main application asset bundle.
    #[must_use]
    pub const fn main() -> Self {
        Self { prefix: "" }
    }

    /// Creates a bundle with a logical subdirectory prefix.
    #[must_use]
    pub const fn new(prefix: &'static str) -> Self {
        Self { prefix }
    }

    /// Returns the logical subdirectory prefix for this bundle.
    #[must_use]
    pub const fn prefix(&self) -> &'static str {
        self.prefix
    }

    /// Resolves a logical asset path to a filesystem path.
    ///
    /// # Panics
    ///
    /// Panics when the `WaterUI` assets root cannot be discovered.
    #[must_use]
    pub fn path(&self, logical_path: &str) -> PathBuf {
        let mut root = assets_root()
            .unwrap_or_else(|error| panic!("WaterUI assets root unavailable: {error}"));
        if !self.prefix.is_empty() {
            root.push(self.prefix);
        }
        if !logical_path.is_empty() {
            root.push(logical_path);
        }
        root
    }

    /// Resolves a logical asset path to a file URL.
    #[must_use]
    pub fn url(&self, logical_path: &str) -> Url {
        Url::from_file_path_str(self.path(logical_path).to_string_lossy().into_owned())
    }
}

/// Image asset resolved from a `WaterUI` asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl ImageAsset {
    /// Creates an image asset handle.
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    /// Returns the logical path inside the asset bundle.
    #[must_use]
    pub const fn logical_path(&self) -> &'static str {
        self.logical_path
    }

    /// Returns the bundle that owns this asset.
    #[must_use]
    pub const fn bundle(&self) -> Bundle {
        self.bundle
    }

    /// Resolves this image asset to a file URL.
    #[must_use]
    pub fn url(&self) -> Url {
        self.bundle.url(self.logical_path)
    }
}

impl View for ImageAsset {
    fn body(self, _env: &Environment) -> impl View {
        Photo::new(self.url())
    }
}

/// Video asset resolved from a `WaterUI` asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl VideoAsset {
    /// Creates a video asset handle.
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    /// Resolves this video asset to a file URL.
    #[must_use]
    pub fn url(&self) -> Url {
        self.bundle.url(self.logical_path)
    }

    /// Builds a raw [`Video`] view from this asset.
    #[must_use]
    pub fn raw(self) -> Video {
        Video::new(self.url())
    }

    /// Builds a [`VideoPlayer`] view from this asset.
    #[must_use]
    pub fn player(self) -> VideoPlayer {
        VideoPlayer::new(self.url())
    }
}

impl View for VideoAsset {
    fn body(self, _env: &Environment) -> impl View {
        self.raw()
    }
}

/// Audio asset resolved from a `WaterUI` asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl AudioAsset {
    /// Creates an audio asset handle.
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    /// Resolves this audio asset to a file URL.
    #[must_use]
    pub fn url(&self) -> Url {
        self.bundle.url(self.logical_path)
    }

    /// Resolves this audio asset to a filesystem path.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }
}

/// Small data asset resolved from a `WaterUI` asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl DataAsset {
    /// Creates a data asset handle.
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    /// Loads this data asset into memory.
    ///
    /// # Errors
    ///
    /// Returns [`AssetError`] when the asset cannot be read from disk.
    pub fn load(&self) -> Result<Data, AssetError> {
        Data::from_local(self.bundle.path(self.logical_path))
    }

    /// Resolves this data asset to a filesystem path.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }
}

/// Large file asset resolved from a `WaterUI` asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LargeFileAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl LargeFileAsset {
    /// Creates a large-file asset handle.
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    /// Opens this large file with asynchronous memory-map setup.
    ///
    /// # Errors
    ///
    /// Returns [`AssetError`] when the asset cannot be read or memory-mapped.
    pub async fn load(&self) -> Result<LargeFile, AssetError> {
        LargeFile::from_local(self.bundle.path(self.logical_path)).await
    }

    /// Resolves this large-file asset to a filesystem path.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }
}

/// Font asset resolved from a `WaterUI` asset bundle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl FontAsset {
    /// Creates a font asset handle.
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    /// Resolves this font asset to a filesystem path.
    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }

    /// Returns the logical path inside the asset bundle.
    #[must_use]
    pub const fn logical_path(&self) -> &'static str {
        self.logical_path
    }
}

fn assets_root() -> Result<PathBuf, AssetError> {
    if let Some(root) = env::var_os(ASSETS_ENV) {
        return Ok(PathBuf::from(root));
    }

    let exe = env::current_exe().map_err(|error| AssetError::io(error.to_string()))?;
    let mut candidates = Vec::new();
    if let Some(parent) = exe.parent() {
        candidates.push(parent.join("waterui_assets"));
        candidates.push(parent.join("resources").join("waterui_assets"));
        if let Some(grand_parent) = parent.parent() {
            candidates.push(grand_parent.join("Resources").join("waterui_assets"));
        }
    }

    for candidate in candidates {
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(AssetError::invalid_path(
        Cow::Borrowed(ASSETS_ENV),
        "set WATERUI_ASSETS_ROOT or package assets into a discoverable resources directory",
    ))
}
