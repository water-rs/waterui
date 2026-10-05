//! GPU texture readback for offscreen export paths.
//!
//! Used only by snapshot/export consumers — the headless runtime's
//! [`HeadlessSnapshot`](crate::HeadlessSnapshot) capture,
//! [`HydrolysisViewRenderer`](crate::HydrolysisViewRenderer)'s
//! `render_to_rgba` and `waterui-testing`'s snapshots — never by the
//! interactive frame loop, which stays GPU-resident end to end.

use crate::platform::SurfaceProvider;

/// wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`: texture-copy rows pad to 256 bytes.
const COPY_BYTES_PER_ROW_ALIGNMENT: u64 = 256;

/// Why a texture readback produced no pixels.
#[derive(Debug, thiserror::Error)]
pub enum ReadbackError {
    /// The surface's device was lost, so the texture holds no rendered frame.
    #[error("the GPU device was lost: {reason}")]
    DeviceLost {
        /// The reason the driver gave for the loss.
        reason: String,
    },
    /// Waiting for the copy into the readback buffer failed.
    #[error("waiting for the readback copy failed")]
    Poll(#[source] wgpu::PollError),
    /// The readback buffer could not be mapped.
    #[error("mapping the readback buffer failed")]
    Map(#[source] wgpu::BufferAsyncError),
    /// The mapped readback buffer could not be read.
    #[error("reading the mapped readback buffer failed")]
    MappedRange(#[source] wgpu::MapRangeError),
}

/// Copies `texture`, rendered on `surface`'s device, into tightly packed
/// RGBA8 rows.
///
/// For export and test paths only — never for a runtime render path, which
/// stays GPU-resident end to end.
///
/// # Errors
///
/// Returns [`ReadbackError`] when the device was lost or the copy could not
/// be waited on, mapped or read.
///
/// # Panics
///
/// Panics when a padded row of `width` pixels exceeds `u32::MAX` bytes.
pub fn readback_texture_rgba8(
    surface: &(impl SurfaceProvider + ?Sized),
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, ReadbackError> {
    let (device, queue) = (surface.device(), surface.queue());
    let row_bytes = u64::from(width) * 4;
    let padded_row_bytes =
        row_bytes.div_ceil(COPY_BYTES_PER_ROW_ALIGNMENT) * COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer_size = padded_row_bytes * u64::from(height);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hydrolysis_texture_readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("hydrolysis_texture_readback_encoder"),
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(u32::try_from(padded_row_bytes).expect("row stride fits u32")),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);

    let slice = readback.slice(..);
    let (sender, receiver) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |result| {
        // A readback that failed before the map completed has dropped the
        // receiver; the late result has no reader.
        let _ = sender.send(result);
    });

    let polled = device.poll(wgpu::PollType::wait_indefinitely());
    if let Some(reason) = surface.device_loss().reason() {
        return Err(ReadbackError::DeviceLost { reason });
    }
    polled.map_err(ReadbackError::Poll)?;
    receiver
        .recv()
        .expect("a completed indefinite poll has run the readback map callback")
        .map_err(ReadbackError::Map)?;

    let mapped = slice
        .get_mapped_range()
        .map_err(ReadbackError::MappedRange)?;
    let mut pixels =
        Vec::with_capacity(crate::num_cast::u64_as_usize(row_bytes * u64::from(height)));
    for row in 0..u64::from(height) {
        let start = crate::num_cast::u64_as_usize(row * padded_row_bytes);
        pixels.extend_from_slice(&mapped[start..start + crate::num_cast::u64_as_usize(row_bytes)]);
    }
    drop(mapped);
    readback.unmap();
    Ok(pixels)
}
