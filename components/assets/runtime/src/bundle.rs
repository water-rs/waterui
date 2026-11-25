use std::path::PathBuf;
use waterui_resources::ResourceContext;

#[cfg(feature = "media")]
use waterui_core::Environment;
#[cfg(any(feature = "media", feature = "video"))]
use waterui_core::contract_violation;
#[cfg(feature = "media")]
use waterui_core::reactive::impl_constant;
#[cfg(feature = "media")]
use waterui_media::ResolvePhotoUrl;
use waterui_url::Url;
#[cfg(feature = "video")]
use waterui_video::{IntoVideoSource, MediaItem, VideoSource};

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
/// Pass directly to [`waterui_media::photo`] or [`waterui_media::Photo::new`]
/// to render without manually reading [`ResourceContext`]; only available with
/// the `media` feature.
#[cfg(feature = "media")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

#[cfg(feature = "media")]
impl_constant!(ImageAsset);

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

    /// Resolves this image asset to a filesystem path.
    #[must_use]
    pub fn path(&self, resources: &ResourceContext) -> PathBuf {
        self.bundle.path(resources, self.logical_path)
    }

    /// Loads this image asset's encoded bytes into memory.
    ///
    /// # Errors
    ///
    /// Returns [`AssetError`] when the asset cannot be read from disk.
    pub fn load(&self, resources: &ResourceContext) -> Result<Data, AssetError> {
        Data::from_local(self.path(resources))
    }
}

#[cfg(feature = "media")]
impl ResolvePhotoUrl for ImageAsset {
    type Context = ResourceContext;

    fn context(env: &Environment) -> Option<Self::Context> {
        let Some(resources) = ResourceContext::from_environment(env).cloned() else {
            contract_violation!(
                "an image asset needs the host's ResourceContext in its environment"
            );
            return None;
        };
        Some(resources)
    }

    fn resolve(&self, context: &Self::Context) -> Url {
        self.url(context)
    }
}

/// Video asset resolved from a `WaterUI` asset bundle.
///
/// Pass directly to [`waterui_video::video`] or [`waterui_video::video_player`]
/// to render without manually reading [`ResourceContext`]; only available with
/// the `video` feature.
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

    /// Resolves this video asset to a file URL.
    #[must_use]
    pub fn url(&self, resources: &ResourceContext) -> Url {
        self.bundle.url(resources, self.logical_path)
    }
}

#[cfg(feature = "video")]
impl IntoVideoSource for VideoAsset {
    fn into_video_source(self) -> VideoSource {
        VideoSource::from_env(move |env| {
            let Some(resources) = ResourceContext::from_environment(env) else {
                contract_violation!(
                    "a video asset needs the host's ResourceContext in its environment"
                );
                return None;
            };
            Some(MediaItem::from(self.url(resources)))
        })
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
    fn image_asset_resolves_through_photo_source_and_updates_reactively() {
        use waterui_core::{Binding, Environment, Signal as _};
        use waterui_media::IntoPhotoSource as _;

        let mut env = Environment::new();
        env.insert(ResourceContext::new("/app/assets", "/app/fonts"));

        let hero = ImageAsset::new(Bundle::main(), "hero.png");
        let resolved = hero
            .into_photo_source()
            .resolve(&env)
            .expect("image asset resolves with ResourceContext in env");
        assert_eq!(
            resolved.snapshot(),
            Url::from_file_path_str("/app/assets/hero.png")
        );

        let selected = Binding::new(hero);
        let reactive = selected
            .clone()
            .into_photo_source()
            .resolve(&env)
            .expect("reactive image asset resolves with ResourceContext in env");
        assert_eq!(
            reactive.snapshot(),
            Url::from_file_path_str("/app/assets/hero.png")
        );
        selected.set(ImageAsset::new(Bundle::new("cards"), "alt.png"));
        assert_eq!(
            reactive.snapshot(),
            Url::from_file_path_str("/app/assets/cards/alt.png")
        );
    }

    #[cfg(feature = "video")]
    #[test]
    fn video_asset_resolves_through_video_source() {
        use waterui_core::Environment;

        let mut env = Environment::new();
        env.insert(ResourceContext::new("/app/assets", "/app/fonts"));

        let intro = VideoAsset::new(Bundle::main(), "intro.mp4");
        let item = intro
            .into_video_source()
            .resolve(&env)
            .expect("video asset resolves with ResourceContext in env");
        assert_eq!(
            *item.source(),
            Url::from_file_path_str("/app/assets/intro.mp4")
        );
    }
}
