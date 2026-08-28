//! GPU texture readback for offscreen export paths.
//!
//! Used only by snapshot/export consumers — the headless runtime's
//! [`HeadlessSnapshot`](crate::HeadlessSnapshot) capture and
//! [`HydrolysisViewRenderer`](crate::HydrolysisViewRenderer)'s
//! `render_to_rgba` — never by the interactive frame loop, which stays
//! GPU-resident end to end.

use waterui_graphics::TextureRowLayout;

pub(crate) fn readback_texture_rgba8(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<u8> {
    let layout = TextureRowLayout::rgba8(width, height);

    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("hydrolysis_texture_readback"),
        size: layout.padded_buffer_size(),
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
            layout: layout.buffer_layout(),
        },
        layout.extent(),
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

    let mapped = slice.get_mapped_range();
    let pixels = layout.unpad_rows(&mapped);
    drop(mapped);
    readback.unmap();
    pixels
}
