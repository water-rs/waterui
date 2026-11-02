//! Helpers that turn readbacks into images for visual-equivalence checks.

use cherenkov::Readback;
use cherenkov_oracle::F32Image;

/// A readback as an image.
pub fn image(readback: &Readback) -> F32Image {
    F32Image {
        width: readback.width,
        height: readback.height,
        pixels: readback.pixels.clone(),
    }
}
