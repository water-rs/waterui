//! View renderer utilities for preview.
//!
//! This module re-exports the `ViewRenderer` from `waterui-core` and adds
//! PNG encoding functionality for the preview system.

use std::io::Cursor;

pub use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};

/// Extension trait for `RenderResult` to add PNG encoding.
pub trait RenderResultExt {
    /// Encode the RGBA data as PNG, consuming the render result.
    fn into_png(self) -> Vec<u8>;

    /// Encode the RGBA data as PNG.
    ///
    /// # Panics
    ///
    /// Panics if the buffer size doesn't match width * height * 4.
    fn to_png(&self) -> Vec<u8>;
}

impl RenderResultExt for RenderResult {
    fn into_png(self) -> Vec<u8> {
        encode_png(self.width, self.height, self.rgba_data)
    }

    fn to_png(&self) -> Vec<u8> {
        encode_png(self.width, self.height, self.rgba_data.clone())
    }
}

fn encode_png(width: u32, height: u32, rgba_data: Vec<u8>) -> Vec<u8> {
    use image::{ImageBuffer, Rgba};

    if rgba_data.is_empty() || width == 0 || height == 0 {
        return Vec::new();
    }

    let img: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(width, height, rgba_data).expect("invalid RGBA buffer size");

    let mut png_bytes = Vec::new();
    img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
        .expect("PNG encoding failed");
    png_bytes
}
