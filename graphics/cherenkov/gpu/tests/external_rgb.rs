//! External RGB planes on a shared device, pinned on the native backend
//! the way `browser.rs` pins them on the web: `bgra8unorm` already swizzles
//! at the sampler, so a red texel must read back red, and PQ decode carries
//! an absolute level (a fraction of 10000 nits) into the working space.

use cherenkov::kurbo::Affine;
use cherenkov::{Engine, EngineError, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{ExternalFrame, FrameColor, RgbAlpha, SharedDevice, wgpu},
};

/// An adapter/device pair usable as a `SharedDevice`: passthrough backends
/// need `PASSTHROUGH_SHADERS`, which the engine requires for its precompiled
/// fixed shaders (issue #57).
fn shared_device()
-> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let required_features = match adapter.get_info().backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal => wgpu::Features::PASSTHROUGH_SHADERS,
        _ => wgpu::Features::empty(),
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..wgpu::DeviceDescriptor::default()
    }))?;
    Ok((instance, adapter, device, queue))
}

/// A 4x4 single-mip `TEXTURE_BINDING` plane filled with one texel.
fn plane(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    texel: &[u8],
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("external rgb plane"),
        size: wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &texel.repeat(16),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * u32::try_from(texel.len()).expect("texel size fits")),
            rows_per_image: Some(4),
        },
        wgpu::Extent3d {
            width: 4,
            height: 4,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Renders `frame` into a 16x16 `LinearF16` surface and returns the pixel
/// inside the layer's quad.
fn sample(
    engine: &Engine<Gpu>,
    surface: &cherenkov::Surface<Gpu>,
    frame: ExternalFrame,
) -> Result<[f32; 4], Box<dyn std::error::Error>> {
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(frame);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer]
            .transform(Affine::translate((2.0, 2.0)))
            .content(video.at((4, 4)));
    });
    engine.render(FrameTime::now())?;
    let rb = surface.readback()?;
    Ok(rb.pixels[(3 * rb.width + 3) as usize])
}

#[test]
fn a_bgra_plane_samples_in_rgba_order() -> Result<(), Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = shared_device()?;
    let engine = match Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        ..GpuConfig::default()
    }) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))?;
    // BGRA memory order for red: B=0, G=0, R=255, A=255.
    let red = plane(
        &device,
        &queue,
        wgpu::TextureFormat::Bgra8Unorm,
        &[0, 0, 255, 255],
    );
    let px = sample(
        &engine,
        &surface,
        ExternalFrame::rgb(red, RgbAlpha::Opaque, FrameColor::SRGB)?,
    )?;
    assert!(
        px[0] > 0.8 && px[1] < 0.1 && px[2] < 0.2 && px[3] > 0.99,
        "bgra8unorm samples are already RGBA-ordered: {px:?}"
    );
    Ok(())
}

#[test]
fn a_pq_frame_carries_an_absolute_level() -> Result<(), Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = shared_device()?;
    let engine = match Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        ..GpuConfig::default()
    }) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16))?;
    // A 0.75 ST 2084 signal decodes to ~984 nits; the working space is
    // relative to reference white (203), so ~4.8 linear.
    let mut texel = Vec::with_capacity(8);
    for channel in [0.75_f32, 0.75, 0.75, 1.0] {
        texel.extend_from_slice(&half::f16::from_f32(channel).to_le_bytes());
    }
    let grey = plane(&device, &queue, wgpu::TextureFormat::Rgba16Float, &texel);
    let px = sample(
        &engine,
        &surface,
        ExternalFrame::rgb(grey, RgbAlpha::Opaque, FrameColor::BT2020_PQ)?,
    )?;
    assert!(
        px[0] > 3.0 && (px[0] - px[1]).abs() < 0.1 && (px[1] - px[2]).abs() < 0.1,
        "pq decodes to an absolute level above SDR white: {px:?}"
    );
    Ok(())
}
