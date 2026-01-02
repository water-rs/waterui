//! View renderer utilities for preview.
//!
//! This module re-exports the `ViewRenderer` from `waterui-core` and adds
//! PNG encoding functionality for the preview system.

use std::io::Cursor;

pub use waterui_core::view_renderer::{CustomViewRenderer, RenderResult, RenderSize, ViewRenderer};

/// Extension trait for `RenderResult` to add PNG encoding.
pub trait RenderResultExt {
    /// Encode the RGBA data as PNG.
    ///
    /// # Panics
    ///
    /// Panics if the buffer size doesn't match width * height * 4.
    fn to_png(&self) -> Vec<u8>;
}

impl RenderResultExt for RenderResult {
    fn to_png(&self) -> Vec<u8> {
        use image::{ImageBuffer, Rgba};

        if self.rgba_data.is_empty() || self.width == 0 || self.height == 0 {
            return Vec::new();
        }

        let img: ImageBuffer<Rgba<u8>, _> =
            ImageBuffer::from_raw(self.width, self.height, self.rgba_data.clone())
                .expect("buffer size matches dimensions");

        let mut png_bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut png_bytes), image::ImageFormat::Png)
            .expect("PNG encoding should not fail");

        png_bytes
    }
}
