//! View renderer utilities for preview.
//!
//! This module re-exports the `ViewRenderer` from `waterui-core` and adds
//! PNG encoding functionality for the preview system.

pub use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};

/// Extension trait for `RenderResult` to add PNG encoding.
pub trait RenderResultExt {
    /// Encode the RGBA data as PNG, consuming the render result.
    fn into_png(self) -> Result<Vec<u8>, String>;

    /// Encode the RGBA data as PNG.
    fn to_png(&self) -> Result<Vec<u8>, String>;
}

impl RenderResultExt for RenderResult {
    fn into_png(self) -> Result<Vec<u8>, String> {
        encode_png(self.width, self.height, self.rgba_data)
    }

    fn to_png(&self) -> Result<Vec<u8>, String> {
        encode_png(self.width, self.height, self.rgba_data.clone())
    }
}

fn encode_png(width: u32, height: u32, rgba_data: Vec<u8>) -> Result<Vec<u8>, String> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::{ExtendedColorType, ImageBuffer, ImageEncoder, Rgba};

    if rgba_data.is_empty() || width == 0 || height == 0 {
        return Ok(Vec::new());
    }

    let Some(img): Option<ImageBuffer<Rgba<u8>, _>> =
        ImageBuffer::from_raw(width, height, rgba_data)
    else {
        return Err("invalid RGBA buffer size".to_string());
    };

    let mut png_bytes = Vec::new();
    // Preview favors encode speed over smallest file size.
    let encoder =
        PngEncoder::new_with_quality(&mut png_bytes, CompressionType::Fast, FilterType::NoFilter);
    encoder
        .write_image(img.as_raw(), width, height, ExtendedColorType::Rgba8)
        .map_err(|e| format!("PNG encoding failed: {e}"))?;
    Ok(png_bytes)
}
