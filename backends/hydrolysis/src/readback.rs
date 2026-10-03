//! GPU texture readback for offscreen export paths.
//!
//! Used only by snapshot/export consumers — the headless runtime's
//! [`HeadlessSnapshot`](crate::HeadlessSnapshot) capture and
//! [`HydrolysisViewRenderer`](crate::HydrolysisViewRenderer)'s
//! `render_to_rgba` — never by the interactive frame loop, which stays
//! GPU-resident end to end.

/// wgpu's `COPY_BYTES_PER_ROW_ALIGNMENT`: texture-copy rows pad to 256 bytes.
const COPY_BYTES_PER_ROW_ALIGNMENT: u64 = 256;

pub(crate) fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
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
        sender
            .send(result)
            .expect("hydrolysis texture readback callback receiver dropped");
    });

    let _ = device.poll(wgpu::PollType::wait_indefinitely());
    receiver
        .recv()
        .expect("hydrolysis texture readback callback dropped")
        .expect("hydrolysis failed to map texture readback buffer");

    let mapped = slice
        .get_mapped_range()
        .expect("hydrolysis failed to read the mapped readback buffer");
    let mut pixels = Vec::with_capacity((row_bytes * u64::from(height)) as usize);
    for row in 0..u64::from(height) {
        let start = (row * padded_row_bytes) as usize;
        pixels.extend_from_slice(&mapped[start..start + row_bytes as usize]);
    }
    drop(mapped);
    readback.unmap();
    pixels
}
