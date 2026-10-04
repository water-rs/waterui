//! Shared-device native presentation preserves extended color and resize ownership.

use cherenkov::{
    __engine_block as block, __engine_fn as split_fn, __engine_test as split_test,
    __engine_wait as wait,
};
use cherenkov::{Engine, FrameTime, WorkingColor};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{
        OutputAlpha, OutputColor, Presenter, SharedDevice, TextureOutput, TextureTarget,
        shader_delivery, wgpu,
    },
};

split_fn! {
/// An adapter/device pair usable as a `SharedDevice`: passthrough backends
/// need `PASSTHROUGH_SHADERS`, which the engine requires for its precompiled
/// fixed shaders (issue #57).
fn shared_device()
-> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        block!(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let required_features = match adapter.get_info().backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal => wgpu::Features::PASSTHROUGH_SHADERS,
        _ => wgpu::Features::empty(),
    };
    let (device, queue) = block!(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..wgpu::DeviceDescriptor::default()
    }))?;
    Ok((instance, adapter, device, queue))
}
}

split_test! {
fn exported_texture_preserves_hdr_and_updates_after_resize()
-> Result<(), Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = wait!(shared_device())?;
    let backend = adapter.get_info().backend;
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        ..GpuConfig::default()
    }))?;
    let (target, textures) = TextureTarget::new((16, 16));
    let source = wait!(engine.surface(target))?;
    source.clear_color(WorkingColor::new([4.0, 0.5, 0.0, 0.5]));
    let texture = textures.try_recv()?;
    let (target, destinations) = TextureTarget::new((16, 16));
    let destination = wait!(engine.surface(target))?;
    let output = destinations.try_recv()?;
    wait!(engine.render(FrameTime::now()))?;
    let delivery = shader_delivery(backend, &device)?;
    let mut presenter = Presenter::new(&device, delivery);
    presenter.texture(
        &device,
        &queue,
        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
        TextureOutput {
            texture: &output,
            color: OutputColor::LinearDisplayP3,
            alpha: OutputAlpha::Premultiplied,
            headroom: 4.0,
        },
    );
    let pixel = wait!(destination.readback())?.pixels[0];
    // The stored texel is premultiplied [2, 0.25, 0, 0.5] — straight
    // (4, 0.5, 0). The tone map compresses it towards headroom 4 by a
    // per-pixel scalar (#97): the HDR channel stays extended (never
    // clipped) and the hue ratio survives.
    let straight = cherenkov_oracle::tone::tone_map(4.0, [4.0, 0.5, 0.0]);
    for (actual, expected) in
        pixel
            .into_iter()
            .zip([straight[0] * 0.5, straight[1] * 0.5, 0.0, 0.5])
    {
        assert!(
            (f64::from(actual) - expected).abs() < 0.001,
            "native output {actual} != {expected}"
        );
    }
    assert!(pixel[0] > 1.0, "extended output keeps HDR range: {pixel:?}");
    source.clear_color(WorkingColor::new([1.0, 1.0, 1.0, 0.5]));
    wait!(engine.render(FrameTime::now()))?;
    presenter.texture(
        &device,
        &queue,
        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
        TextureOutput {
            texture: &output,
            color: OutputColor::Srgb,
            alpha: OutputAlpha::Premultiplied,
            headroom: 4.0,
        },
    );
    let pixel = wait!(destination.readback())?.pixels[0];
    for actual in pixel {
        assert!(
            (actual - 0.5).abs() < 0.001,
            "sRGB premultiplication follows transfer encoding: {actual}"
        );
    }
    source.resize((8, 4))?;
    wait!(engine.render(FrameTime::now()))?;
    let resized = textures.try_recv()?;
    assert_eq!((resized.width(), resized.height()), (8, 4));
    assert_eq!(
        (texture.width(), texture.height()),
        (16, 16),
        "host retains old texture until released"
    );
    assert!(
        textures.try_recv().is_err(),
        "only allocation changes publish a texture"
    );
    Ok(())
}
}

split_test! {
fn hardware_and_shader_srgb_store_the_same_premultiplied_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = wait!(shared_device())?;
    let backend = adapter.get_info().backend;
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        ..GpuConfig::default()
    }))?;
    let (target, textures) = TextureTarget::new((1, 1));
    let surface = wait!(engine.surface(target))?;
    let source = textures.recv()?;
    let view = source.create_view(&wgpu::TextureViewDescriptor::default());
    let delivery = shader_delivery(backend, &device)?;
    let mut presenter = Presenter::new(&device, delivery);
    let outputs = [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ]
    .map(|format| presentation_target(&device, format));
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("presentation readback"),
        size: 512,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    for alpha in [0.0, 0.25, 0.5, 1.0] {
        surface.clear_color(WorkingColor::new([1.0, 1.0, 1.0, alpha]));
        wait!(engine.render(FrameTime::now()))?;
        for output in &outputs {
            presenter.texture(
                &device,
                &queue,
                &view,
                TextureOutput {
                    texture: output,
                    color: OutputColor::Srgb,
                    alpha: OutputAlpha::Premultiplied,
                    headroom: 1.0,
                },
            );
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for (i, output) in outputs.iter().enumerate() {
            encoder.copy_texture_to_buffer(
                output.as_image_copy(),
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: u64::try_from(i)? * 256,
                        bytes_per_row: Some(256),
                        rows_per_image: Some(1),
                    },
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
        }
        let submission = queue.submit([encoder.finish()]);
        let (send, receive) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = send.send(result);
            });
        device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(std::time::Duration::from_secs(30)),
        })?;
        receive.recv()??;
        let bytes = buffer
            .slice(..)
            .get_mapped_range()
            .expect("buffer range is mapped and not overlapping");
        for (a, b) in bytes[..4].iter().zip(&bytes[256..260]) {
            assert!(
                a.abs_diff(*b) <= 1,
                "hardware transfer differs at alpha {alpha}: {a} vs {b}"
            );
            assert!((f32::from(*a) / 255.0 - alpha).abs() < 0.005);
        }
        drop(bytes);
        buffer.unmap();
    }
    Ok(())
}
}

fn presentation_target(device: &wgpu::Device, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("presentation test"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}
