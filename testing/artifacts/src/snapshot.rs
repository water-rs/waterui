use std::path::Path;

/// RGBA8 frame captured from a headless render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Pixel data in RGBA8 row-major order.
    pub rgba8: Vec<u8>,
}

impl Snapshot {
    /// Saves the snapshot to a PNG file.
    ///
    /// # Errors
    ///
    /// Returns image I/O or encoding errors from the underlying PNG writer.
    ///
    /// # Panics
    ///
    /// Panics if the stored RGBA buffer length does not match the snapshot dimensions.
    pub fn save_png(&self, path: impl AsRef<Path>) -> image::ImageResult<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(image::ImageError::IoError)?;
        }
        let image = image::RgbaImage::from_raw(self.width, self.height, self.rgba8.clone())
            .expect("Snapshot::save_png: rgba buffer shape must match dimensions");
        image.save(path)
    }
}
