//! The present pass's gamut mapping against the `f64` oracle (#96): a
//! row of linear Display P3 pixels — in-gamut, P3 primaries and
//! secondaries, crossing a gradient, HDR — is presented through the
//! shader-encode sRGB path and compared to the oracle's map quantized to
//! unorm-8. In-gamut pixels must match the pre-#96 clamp result exactly.

use cherenkov::{
    __engine_block as block, __engine_fn as split_fn, __engine_test as split_test,
    __engine_wait as wait,
};
use cherenkov_gpu::interop::{
    OutputAlpha, OutputColor, Presenter, TextureOutput, shader_delivery, wgpu,
};
use cherenkov_oracle::color::srgb_encode;
use cherenkov_oracle::gamut::gamut_map_srgb_analytic;

/// The present shader's simplified P3 → linear sRGB matrix (the same
/// constants, in `f64`).
const P3_TO_LINEAR_SRGB: [[f64; 3]; 3] = [
    [1.224_940_2, -0.224_940_2, 0.0],
    [-0.042_056_95, 1.042_056_9, 0.0],
    [-0.019_637_55, -0.078_636_05, 1.098_273_6],
];

split_fn! {
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

/// The oracle's expected stored byte for one opaque working-space pixel:
/// the headroom tone map in P3 (#97), P3 -> linear sRGB, the analytic
/// gamut map, encode, round to unorm8.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "mapped channels are in [0,1]; the unorm-8 store rounds"
)]
fn expected_bytes(headroom: f64, p3: [f64; 3]) -> [u8; 4] {
    // An sRGB destination's ceiling caps the tone-map target at 1, and
    // the shoulder runs in the working space.
    let p3 = cherenkov_oracle::tone::tone_map(headroom.min(1.0), p3);
    let srgb: [f64; 3] = P3_TO_LINEAR_SRGB
        .iter()
        .map(|row| row[0].mul_add(p3[0], row[1].mul_add(p3[1], row[2] * p3[2])))
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let in_gamut = srgb.iter().all(|&c| (0.0..=1.0).contains(&c));
    let mapped = gamut_map_srgb_analytic(srgb);
    let byte = |c: f64| (srgb_encode(c).clamp(0.0, 1.0) * 255.0).round() as u8;
    let b = mapped.map(byte);
    // For in-gamut pixels the map is the identity, so `expected` equals
    // plain convert + clamp — assert that contract while we're here.
    if in_gamut {
        let plain = srgb.map(|c| byte(c.clamp(0.0, 1.0)));
        assert_eq!(b, plain, "in-gamut pixel altered: {srgb:?}");
    }
    [b[0], b[1], b[2], 255]
}

split_test! {
fn present_gamut_map_matches_oracle() -> Result<(), Box<dyn std::error::Error>> {
    // Opaque premultiplied = straight pixels.
    let pixels: [[f32; 4]; 12] = [
        [0.5, 0.5, 0.5, 1.0],   // in-gamut grey
        [0.25, 0.5, 0.75, 1.0], // in-gamut colour
        [1.0, 0.0, 0.0, 1.0],   // P3 red
        [0.0, 1.0, 0.0, 1.0],   // P3 green
        [0.0, 0.0, 1.0, 1.0],   // P3 blue
        [0.0, 1.0, 1.0, 1.0],   // P3 cyan
        [1.0, 0.0, 1.0, 1.0],   // P3 magenta
        [1.0, 1.0, 0.0, 1.0],   // P3 yellow
        [0.95, 0.3, 0.15, 1.0], // crossing the boundary
        [4.0, 4.0, 4.0, 1.0],   // HDR white
        [-0.1, 0.5, 0.5, 1.0],  // negative channel
        [1.2, 0.9, 0.4, 1.0],   // bright orange, partly out
    ];
    // An in-sRGB colour round-tripped through the P3 matrices lands a few
    // ULPs out of gamut; the map must keep the pre-#96 bytes. Asserted
    // separately because the field is not representable as a literal.
    let ulp: [[f32; 4]; 1] = [[-7e-18, -3e-17, 1.0, 1.0]];
    let pixels: Vec<[f32; 4]> = pixels.iter().copied().chain(ulp).collect();
    let (w, h) = (u32::try_from(pixels.len()).unwrap(), 1);
    let (_instance, adapter, device, queue) = wait!(shared_device())?;

    let source = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("gamut source"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let mut data = Vec::new();
    for p in &pixels {
        for c in p {
            data.extend_from_slice(&half::f16::from_f32(*c).to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &source,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(w * 8),
            rows_per_image: Some(h),
        },
        source.size(),
    );
    let delivery = shader_delivery(adapter.get_info().backend, &device)?;
    let mut presenter = Presenter::new(&device, delivery);
    // SDR and HDR display headrooms: the >1 pixels take the tone-map
    // branch at both (#97).
    let headrooms = [1.0f32, 2.0];
    let outputs: Vec<wgpu::Texture> = headrooms
        .iter()
        .map(|_| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gamut output"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        })
        .collect();
    let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
    for (output, &headroom) in outputs.iter().zip(&headrooms) {
        presenter.texture(
            &device,
            &queue,
            &source_view,
            TextureOutput {
                texture: output,
                color: OutputColor::Srgb,
                alpha: OutputAlpha::Premultiplied,
                headroom,
            },
        );
    }
    let row = (w * 4).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("gamut readback"),
        size: u64::from(row * h) * outputs.len() as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    for (j, output) in outputs.iter().enumerate() {
        encoder.copy_texture_to_buffer(
            output.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: j as u64 * u64::from(row * h),
                    bytes_per_row: Some(row),
                    rows_per_image: Some(h),
                },
            },
            output.size(),
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
    for (j, &headroom) in headrooms.iter().enumerate() {
        let base = j * usize::try_from(row * h)?;
        for (i, p) in pixels.iter().enumerate() {
            let got = &bytes[base + i * 4..base + i * 4 + 4];
            let want = expected_bytes(
                f64::from(headroom),
                <[f64; 3]>::try_from(p[..3].iter().map(|&c| f64::from(c)).collect::<Vec<_>>())
                    .unwrap(),
            );
            for (g, w) in got.iter().zip(want) {
                // f32 shader vs f64 oracle: same algorithm, ±2 LSB of
                // unorm-8 for rounding at step boundaries.
                assert!(
                    g.abs_diff(w) <= 2,
                    "headroom {headroom} pixel {i} {p:?}: shader {got:?} vs oracle {want:?}"
                );
            }
        }
    }
    Ok(())
}
}
