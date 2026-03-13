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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Bundle {
    prefix: &'static str,
}

impl Bundle {
    #[must_use]
    pub const fn main() -> Self {
        Self { prefix: "" }
    }

    #[must_use]
    pub const fn new(prefix: &'static str) -> Self {
        Self { prefix }
    }

    #[must_use]
    pub const fn prefix(&self) -> &'static str {
        self.prefix
    }

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

    #[must_use]
    pub fn url(&self, logical_path: &str) -> Url {
        Url::from_file_path_str(self.path(logical_path).to_string_lossy().into_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl ImageAsset {
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    #[must_use]
    pub const fn logical_path(&self) -> &'static str {
        self.logical_path
    }

    #[must_use]
    pub const fn bundle(&self) -> Bundle {
        self.bundle
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl VideoAsset {
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    #[must_use]
    pub fn url(&self) -> Url {
        self.bundle.url(self.logical_path)
    }

    #[must_use]
    pub fn raw(self) -> Video {
        Video::new(self.url())
    }

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AudioAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl AudioAsset {
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    #[must_use]
    pub fn url(&self) -> Url {
        self.bundle.url(self.logical_path)
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DataAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl DataAsset {
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    pub fn load(&self) -> Result<Data, AssetError> {
        Data::from_local(self.bundle.path(self.logical_path))
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LargeFileAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl LargeFileAsset {
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    pub async fn load(&self) -> Result<LargeFile, AssetError> {
        LargeFile::from_local(self.bundle.path(self.logical_path)).await
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FontAsset {
    bundle: Bundle,
    logical_path: &'static str,
}

impl FontAsset {
    #[must_use]
    pub const fn new(bundle: Bundle, logical_path: &'static str) -> Self {
        Self {
            bundle,
            logical_path,
        }
    }

    #[must_use]
    pub fn path(&self) -> PathBuf {
        self.bundle.path(self.logical_path)
    }

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
