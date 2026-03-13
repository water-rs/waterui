//! Asset type classification based on file extension.

/// Classification of asset types based on file extension.
///
/// Used by the `asset!` macro to determine the appropriate return type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    /// Font files (.ttf, .otf, .ttc, .otc, .woff, .woff2)
    /// Returns: `FontAsset`
    Font,

    /// Image files (.png, .jpg, .jpeg, .gif, .webp, .avif)
    /// Returns: `Photo`
    Image,

    /// Video files (.mp4, .mov, .webm, .avi)
    /// Returns: `Video`
    Video,

    /// Audio files (.mp3, .wav, .ogg, .flac, .aac)
    /// Returns: `Audio`
    Audio,

    /// Large binary files (.onnx, .bin, .safetensors, .gguf)
    /// Returns: `LargeFile` (memory-mapped, always async)
    LargeModel,

    /// Small binary files (all other extensions)
    /// Returns: `Data` (loaded into memory)
    Data,
}

impl AssetKind {
    /// Infer asset kind from file extension.
    ///
    /// # Examples
    ///
    /// ```
    /// use waterui_assets::AssetKind;
    ///
    /// assert_eq!(AssetKind::from_extension("png"), AssetKind::Image);
    /// assert_eq!(AssetKind::from_extension("mp4"), AssetKind::Video);
    /// assert_eq!(AssetKind::from_extension("onnx"), AssetKind::LargeModel);
    /// assert_eq!(AssetKind::from_extension("json"), AssetKind::Data);
    /// ```
    #[must_use]
    pub const fn from_extension(ext: &str) -> Self {
        // Note: const fn cannot use match on &str directly in stable Rust,
        // so we use a helper approach
        match ext.as_bytes() {
            // Font extensions
            b"ttf" | b"otf" | b"ttc" | b"otc" | b"woff" | b"woff2" => Self::Font,
            // Image extensions
            b"png" | b"jpg" | b"jpeg" | b"gif" | b"webp" | b"avif" | b"bmp" | b"ico" | b"svg" => {
                Self::Image
            }
            // Video extensions
            b"mp4" | b"mov" | b"webm" | b"avi" | b"mkv" => Self::Video,
            // Audio extensions
            b"mp3" | b"wav" | b"ogg" | b"flac" | b"aac" | b"m4a" => Self::Audio,
            // Large model extensions (ML models, large binaries)
            b"onnx" | b"safetensors" | b"gguf" | b"ggml" | b"pt" | b"pth" | b"bin" => {
                Self::LargeModel
            }
            // Everything else is Data
            _ => Self::Data,
        }
    }

    /// Check if this asset kind requires async loading.
    ///
    /// - `LargeModel` always requires async (mmap setup)
    /// - `Data` requires async only for remote URLs
    /// - Media types (Image, Video, Audio) are always sync (URL-based)
    #[must_use]
    pub const fn requires_async_local(&self) -> bool {
        matches!(self, Self::LargeModel)
    }

    /// Check if this asset kind supports embed mode.
    ///
    /// Only `Data` supports `embed = true` in the `asset!` macro.
    #[must_use]
    pub const fn supports_embed(&self) -> bool {
        matches!(self, Self::Data)
    }

    #[must_use]
    pub const fn is_font(&self) -> bool {
        matches!(self, Self::Font)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_image_extensions() {
        assert_eq!(AssetKind::from_extension("png"), AssetKind::Image);
        assert_eq!(AssetKind::from_extension("jpg"), AssetKind::Image);
        assert_eq!(AssetKind::from_extension("jpeg"), AssetKind::Image);
        assert_eq!(AssetKind::from_extension("gif"), AssetKind::Image);
        assert_eq!(AssetKind::from_extension("webp"), AssetKind::Image);
        assert_eq!(AssetKind::from_extension("svg"), AssetKind::Image);
    }

    #[test]
    fn test_video_extensions() {
        assert_eq!(AssetKind::from_extension("mp4"), AssetKind::Video);
        assert_eq!(AssetKind::from_extension("mov"), AssetKind::Video);
        assert_eq!(AssetKind::from_extension("webm"), AssetKind::Video);
    }

    #[test]
    fn test_audio_extensions() {
        assert_eq!(AssetKind::from_extension("mp3"), AssetKind::Audio);
        assert_eq!(AssetKind::from_extension("wav"), AssetKind::Audio);
        assert_eq!(AssetKind::from_extension("ogg"), AssetKind::Audio);
    }

    #[test]
    fn test_font_extensions() {
        assert_eq!(AssetKind::from_extension("ttf"), AssetKind::Font);
        assert_eq!(AssetKind::from_extension("otf"), AssetKind::Font);
        assert_eq!(AssetKind::from_extension("woff2"), AssetKind::Font);
    }

    #[test]
    fn test_large_model_extensions() {
        assert_eq!(AssetKind::from_extension("onnx"), AssetKind::LargeModel);
        assert_eq!(
            AssetKind::from_extension("safetensors"),
            AssetKind::LargeModel
        );
        assert_eq!(AssetKind::from_extension("gguf"), AssetKind::LargeModel);
        assert_eq!(AssetKind::from_extension("bin"), AssetKind::LargeModel);
    }

    #[test]
    fn test_data_extensions() {
        assert_eq!(AssetKind::from_extension("json"), AssetKind::Data);
        assert_eq!(AssetKind::from_extension("toml"), AssetKind::Data);
        assert_eq!(AssetKind::from_extension("wgsl"), AssetKind::Data);
        assert_eq!(AssetKind::from_extension("txt"), AssetKind::Data);
    }
}
