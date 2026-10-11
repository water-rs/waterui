use std::path::PathBuf;
use waterui_core::ResourceContext;

#[cfg(any(feature = "media", feature = "video"))]
use waterui_core::{Environment, View};
#[cfg(feature = "media")]
use waterui_media::Photo;
use waterui_url::Url;
#[cfg(feature = "video")]
use waterui_video::{Video, VideoPlayer};

use crate::{AssetError, Data, LargeFile};

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
    #[must_use]
    pub fn path(&self, resources: &ResourceContext, logical_path: &str) -> PathBuf {
        let mut root = resources.assets().to_path_buf();
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
    pub fn url(&self, resources: &ResourceContext, logical_path: &str) -> Url {
        Url::from_file_path_str(
            self.path(resources, logical_path)
                .to_string_lossy()
                .into_owned(),
        )
    }
}

/// Image asset resolved from a `WaterUI` asset bundle.
///
/// Renders through [`Photo`]; only available with the `media` feature.
#[cfg(feature = "media")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

#[cfg(feature = "media")]
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
    pub fn url(&self, resources: &ResourceContext) -> Url {
        self.bundle.url(resources, self.logical_path)
    }
}

#[cfg(feature = "media")]
impl View for ImageAsset {
    fn body(self, env: &Environment) -> impl View {
        Photo::new(self.url(ResourceContext::from_environment(env)))
    }

    /// Matches the non-resizable `Photo` constructed by `body`, independent of its URL.
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::None
    }
}

/// Video asset resolved from a `WaterUI` asset bundle.
///
/// Builds [`Video`]/[`VideoPlayer`] views; only available with the `video`
/// feature.
#[cfg(feature = "video")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

#[cfg(feature = "video")]
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
    pub fn url(&self, resources: &ResourceContext) -> Url {
        self.bundle.url(resources, self.logical_path)
    }

    /// Builds a raw [`Video`] view from this asset.
    #[must_use]
    pub fn raw(self, resources: &ResourceContext) -> Video {
        waterui_video::video(self.url(resources))
    }

    /// Builds a [`VideoPlayer`] view from this asset.
    #[must_use]
    pub fn player(self, resources: &ResourceContext) -> VideoPlayer {
        waterui_video::video_player(self.url(resources))
    }
}

#[cfg(feature = "video")]
impl View for VideoAsset {
    fn body(self, env: &Environment) -> impl View {
        self.raw(ResourceContext::from_environment(env))
    }

    /// Matches the default fit-mode `Video` constructed by `body`, independent of its URL.
    fn stretch_axis(&self) -> waterui_core::layout::StretchAxis {
        waterui_core::layout::StretchAxis::Horizontal
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
    pub fn url(&self, resources: &ResourceContext) -> Url {
        self.bundle.url(resources, self.logical_path)
    }

    /// Resolves this audio asset to a filesystem path.
    #[must_use]
    pub fn path(&self, resources: &ResourceContext) -> PathBuf {
        self.bundle.path(resources, self.logical_path)
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
    pub fn load(&self, resources: &ResourceContext) -> Result<Data, AssetError> {
        Data::from_local(self.bundle.path(resources, self.logical_path))
    }

    /// Resolves this data asset to a filesystem path.
    #[must_use]
    pub fn path(&self, resources: &ResourceContext) -> PathBuf {
        self.bundle.path(resources, self.logical_path)
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
    pub async fn load(&self, resources: &ResourceContext) -> Result<LargeFile, AssetError> {
        LargeFile::from_local(self.bundle.path(resources, self.logical_path)).await
    }

    /// Resolves this large-file asset to a filesystem path.
    #[must_use]
    pub fn path(&self, resources: &ResourceContext) -> PathBuf {
        self.bundle.path(resources, self.logical_path)
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
    pub fn path(&self, resources: &ResourceContext) -> PathBuf {
        self.bundle.path(resources, self.logical_path)
    }

    /// Returns the logical path inside the asset bundle.
    #[must_use]
    pub const fn logical_path(&self) -> &'static str {
        self.logical_path
    }
}

/// Root directory owned by this application or embedded instance.
#[must_use]
pub fn bundle_root(resources: &ResourceContext) -> &std::path::Path {
    resources.assets()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn the_same_handle_resolves_against_its_owning_instance() {
        let first = ResourceContext::new("/first/assets", "/first/fonts");
        let second = ResourceContext::new("/second/assets", "/second/fonts");
        let asset = DataAsset::new(Bundle::new("documents"), "readme.json");
        assert_eq!(
            asset.path(&first),
            Path::new("/first/assets/documents/readme.json")
        );
        assert_eq!(
            asset.path(&second),
            Path::new("/second/assets/documents/readme.json")
        );
        assert_eq!(bundle_root(&first), Path::new("/first/assets"));
    }

    #[cfg(feature = "media")]
    #[test]
    fn image_asset_axis_matches_the_resolved_photo() {
        let context = ResourceContext::new("/fixture/assets", "/fixture/fonts");
        let image = ImageAsset::new(Bundle::main(), "image.png");
        assert_eq!(
            image.stretch_axis(),
            Photo::new(image.url(&context)).stretch_axis()
        );
    }

    #[cfg(feature = "video")]
    #[test]
    fn video_asset_axis_matches_the_resolved_video() {
        let context = ResourceContext::new("/fixture/assets", "/fixture/fonts");
        let video = VideoAsset::new(Bundle::main(), "movie.mp4");
        assert_eq!(video.stretch_axis(), video.raw(&context).stretch_axis());
    }
}
