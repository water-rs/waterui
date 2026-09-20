//! Screenshot post-processing: compositing onto an opaque background and PNG
//! encoding.

use image::ImageEncoder as _;
use image::codecs::png::PngEncoder;
use waterui_testing::Snapshot;

/// Composites a straight-alpha RGBA8 buffer over opaque white in place.
///
/// Screenshots ship without alpha: they are flattened for preview UIs and
/// model vision inputs that assume opaque frames.
pub fn flatten_alpha_over_white(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        let alpha = u32::from(pixel[3]);
        if alpha == 255 {
            continue;
        }
        for channel in &mut pixel[..3] {
            *channel = u8::try_from((u32::from(*channel) * alpha + 255 * (255 - alpha)) / 255)
                .expect("alpha composite is bounded by 255");
        }
        pixel[3] = 255;
    }
}

/// Encodes a frame as PNG bytes.
///
/// # Errors
///
/// Returns the `image` error when the buffer size does not match the frame's
/// dimensions or the encoder fails.
pub fn encode(snapshot: &Snapshot) -> image::ImageResult<Vec<u8>> {
    let mut bytes = Vec::new();
    PngEncoder::new(&mut bytes).write_image(
        &snapshot.rgba8,
        snapshot.width,
        snapshot.height,
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(bytes)
}
