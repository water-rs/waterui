//! The present pass's output encodings against the `f64` oracle (#98): a
//! row of working-space pixels — neutral 0/0.18/1/2/4/8, P3 primaries
//! outside sRGB, negative extended components, partially transparent
//! edges, above-white highlights — is presented through every
//! [`OutputColor`] encoding and compared to the oracle's presentation
//! functions in the destination's stored units.

use cherenkov_gpu::interop::{
    OutputAlpha, OutputColor, Presenter, TextureOutput, shader_delivery, wgpu,
};
use cherenkov_oracle::{Image, present};

/// The corpus pixels every encoding presents (premultiplied linear
/// Display P3).
fn pixels() -> Vec<[f32; 4]> {
    vec![
        [0.0, 0.0, 0.0, 1.0],    // neutral 0
        [0.18, 0.18, 0.18, 1.0], // neutral 0.18
        [1.0, 1.0, 1.0, 1.0],    // neutral 1 (SDR white)
        [2.0, 2.0, 2.0, 1.0],    // neutral 2
        [4.0, 4.0, 4.0, 1.0],    // neutral 4
        [8.0, 8.0, 8.0, 1.0],    // neutral 8
        [1.0, 0.0, 0.0, 1.0],    // P3 red — outside sRGB
        [0.0, 1.0, 0.0, 1.0],    // P3 green
        [0.0, 0.0, 1.0, 1.0],    // P3 blue
        [1.0, 0.0, 0.5, 1.0],    // saturated P3 rose
        [-0.25, 0.5, 0.6, 1.0],  // negative extended component
        [0.1, -0.1, 0.3, 1.0],   // negative extended component
        [0.4, 0.2, 0.1, 0.5],    // transparent coloured edge
        [0.1, 0.1, 0.1, 0.25],   // faint edge
        [1.6, 1.6, 1.6, 0.8],    // glass highlight above SDR white
        [3.0, 1.5, 0.4, 1.0],    // HDR orange
    ]
}

fn shared_device() -> Result<(wgpu::Adapter, wgpu::Device, wgpu::Queue), Box<dyn std::error::Error>>
{
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
    Ok((adapter, device, queue))
}

fn f16_texture(device: &wgpu::Device, queue: &wgpu::Queue, pixels: &[[f32; 4]]) -> wgpu::Texture {
    let (w, h) = (u32::try_from(pixels.len()).unwrap(), 1);
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("outputs source"),
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
    for p in pixels {
        for c in p {
            data.extend_from_slice(&half::f16::from_f32(*c).to_le_bytes());
        }
    }
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
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
        texture.size(),
    );
    texture
}

/// The oracle's stored value for pixel `p` under `color` at `headroom` —
/// premultiplied in the destination encoding, as the surfaces store it.
fn oracle_present(color: OutputColor, headroom: f64, pixels: &[[f32; 4]]) -> Image {
    let image = Image {
        width: pixels.len(),
        height: 1,
        pixels: pixels.iter().map(|p| p.map(f64::from)).collect(),
    };
    match color {
        OutputColor::Srgb => present::present_srgb(headroom, &image),
        OutputColor::DisplayP3 => present::present_display_p3(headroom, &image),
        OutputColor::LinearDisplayP3 => present::present_linear_p3(headroom, &image),
        OutputColor::ExtendedSrgbLinear => present::present_extended_srgb_linear(headroom, &image),
        OutputColor::ExtendedSrgb => present::present_extended_srgb(headroom, &image),
        OutputColor::ExtendedDisplayP3 => present::present_extended_display_p3(headroom, &image),
        OutputColor::Bt2100Pq => present::present_pq(headroom, &image),
        OutputColor::Bt2100Hlg => present::present_hlg(headroom, &image),
    }
}

/// Runs the present pass over `pixels` into `format`/`color`/`alpha` and
/// returns the stored values as f64.
#[expect(
    clippy::too_many_arguments,
    reason = "one helper runs the whole pass: device triple, source, output shape and the corpus row"
)]
fn present_gpu(
    adapter: &wgpu::Adapter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    source: &wgpu::Texture,
    format: wgpu::TextureFormat,
    color: OutputColor,
    alpha: OutputAlpha,
    headroom: f32,
    pixels: &[[f32; 4]],
) -> Result<Vec<[f64; 4]>, Box<dyn std::error::Error>> {
    let (w, h) = (u32::try_from(pixels.len()).unwrap(), 1);
    let output = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("outputs target"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let source_view = source.create_view(&wgpu::TextureViewDescriptor::default());
    let mut presenter =
        Presenter::new(device, shader_delivery(adapter.get_info().backend, device)?);
    presenter.texture(
        device,
        queue,
        &source_view,
        TextureOutput {
            texture: &output,
            color,
            alpha,
            headroom,
        },
    );
    let texel = format.block_copy_size(None).unwrap();
    let row = (w * texel).div_ceil(256) * 256;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("outputs readback"),
        size: u64::from(row * h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        output.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(row),
                rows_per_image: Some(h),
            },
        },
        output.size(),
    );
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
    let bytes = buffer.slice(..).get_mapped_range()?;
    let mut out = Vec::with_capacity(pixels.len());
    for i in 0..pixels.len() {
        let base = i * usize::try_from(texel)?;
        out.push(if format.is_srgb() || texel == 4 {
            // unorm-8 channels
            let px: [u8; 4] = bytes[base..base + 4].try_into().unwrap();
            px.map(|b| f64::from(b) / 255.0)
        } else {
            // f16 channels
            let mut px = [0.0; 4];
            for (c, ch) in px.iter_mut().enumerate() {
                let start = base + c * 2;
                *ch = f64::from(half::f16::from_le_bytes(
                    bytes[start..start + 2].try_into().unwrap(),
                ));
            }
            px
        });
    }
    Ok(out)
}

/// `want` and `got` in the destination's stored units: ±2 unorm-8 LSB
/// equivalents plus the f16 quantization floor — the f32 shader against
/// the f64 oracle.
fn assert_close(color: OutputColor, pixel: usize, p: [f32; 4], got: [f64; 4], want: [f64; 4]) {
    for c in 0..4 {
        let tol = 0.02f64.mul_add(want[c].abs(), 0.008);
        assert!(
            (got[c] - want[c]).abs() <= tol,
            "{color:?} pixel {pixel} {p:?} channel {c}: shader {} vs oracle {}",
            got[c],
            want[c],
        );
    }
}

#[test]
fn every_output_encoding_matches_the_oracle() -> Result<(), Box<dyn std::error::Error>> {
    let pixels = pixels();
    let (adapter, device, queue) = shared_device()?;
    let source = f16_texture(&device, &queue, &pixels);
    let headroom = 2.0f32;
    // (format, color) — every present-pass encoding. `Srgb` covers both
    // the hardware (`*Srgb` format) and software (plain unorm) encoders.
    let cases: [(wgpu::TextureFormat, OutputColor); 9] = [
        (wgpu::TextureFormat::Rgba8Unorm, OutputColor::Srgb),
        (wgpu::TextureFormat::Rgba8UnormSrgb, OutputColor::Srgb),
        (wgpu::TextureFormat::Rgba8Unorm, OutputColor::DisplayP3),
        (wgpu::TextureFormat::Rgba8UnormSrgb, OutputColor::DisplayP3),
        (
            wgpu::TextureFormat::Rgba16Float,
            OutputColor::LinearDisplayP3,
        ),
        (
            wgpu::TextureFormat::Rgba16Float,
            OutputColor::ExtendedSrgbLinear,
        ),
        (wgpu::TextureFormat::Rgba16Float, OutputColor::ExtendedSrgb),
        (wgpu::TextureFormat::Rgba16Float, OutputColor::Bt2100Pq),
        (wgpu::TextureFormat::Rgba16Float, OutputColor::Bt2100Hlg),
    ];
    for (format, color) in cases {
        let got = present_gpu(
            &adapter,
            &device,
            &queue,
            &source,
            format,
            color,
            OutputAlpha::Premultiplied,
            headroom,
            &pixels,
        )?;
        let want = oracle_present(color, f64::from(headroom), &pixels);
        for (i, (g, w)) in got.iter().zip(&want.pixels).enumerate() {
            assert_close(color, i, pixels[i], *g, *w);
        }
    }
    Ok(())
}

/// One pinned anchor: `(pixel, stored value, per-channel tolerance)`.
type Anchor = (usize, [f64; 4], f64);
/// One pinned case: `(format, color, anchors)`.
type PinnedCase = (wgpu::TextureFormat, OutputColor, &'static [Anchor]);

/// The oracle's exact stored values at headroom 2 for canonical inputs:
/// SDR white's encoding, the 0.18 midtone a transfer-coefficient slip
/// once escaped `assert_close`'s tolerance through, an HDR neutral, and
/// a negative extended component (#98).
const SRGB_018: [f64; 4] = [
    0.461_356_129_500_441_6,
    0.461_356_118_743_022_07,
    0.461_356_129_500_441_6,
    1.0,
];
/// `DisplayP3` at 0.18 — same OETF, exact P3 primaries.
const DISPLAY_P3_018: [f64; 4] = [
    0.461_356_129_500_441_6,
    0.461_356_129_500_441_6,
    0.461_356_129_500_441_6,
    1.0,
];
const PINNED: [PinnedCase; 10] = [
    (
        wgpu::TextureFormat::Rgba8UnormSrgb,
        OutputColor::Srgb,
        &[
            (1, SRGB_018, 0.0045),
            (2, [1.0, 0.999_999_978_020_833, 1.0, 1.0], 0.0045),
        ],
    ),
    (
        wgpu::TextureFormat::Rgba8Unorm,
        OutputColor::Srgb,
        &[(1, SRGB_018, 0.0045)],
    ),
    (
        wgpu::TextureFormat::Rgba8UnormSrgb,
        OutputColor::DisplayP3,
        &[(1, DISPLAY_P3_018, 0.0045)],
    ),
    (
        wgpu::TextureFormat::Rgba8Unorm,
        OutputColor::DisplayP3,
        &[(1, DISPLAY_P3_018, 0.0045)],
    ),
    (
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::LinearDisplayP3,
        &[
            (2, [1.0, 1.0, 1.0, 1.0], 0.0015),
            (3, [1.5, 1.5, 1.5, 1.0], 0.0025),
        ],
    ),
    (
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::ExtendedSrgbLinear,
        &[(
            10,
            [-0.418_705_15, 0.531_542_687_5, 0.624_555_522_5, 1.0],
            0.001,
        )],
    ),
    (
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::ExtendedSrgb,
        &[
            (
                3,
                [
                    1.194_176_534_680_845,
                    1.194_176_508_656_333_4,
                    1.194_176_534_680_845,
                    1.0,
                ],
                0.002,
            ),
            (
                10,
                [
                    -0.679_030_747_736_711_2,
                    0.755_761_942_589_218_6,
                    0.812_109_075_340_857_7,
                    1.0,
                ],
                0.001,
            ),
        ],
    ),
    (
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::ExtendedDisplayP3,
        &[(
            3,
            [
                1.194_176_534_680_844_8,
                1.194_176_534_680_844_8,
                1.194_176_534_680_844_8,
                1.0,
            ],
            0.002,
        )],
    ),
    (
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::Bt2100Pq,
        &[
            (
                1,
                [
                    0.410_896_752_036_804_8,
                    0.410_896_752_013_288_07,
                    0.410_896_752_034_422_2,
                    1.0,
                ],
                0.0006,
            ),
            (
                2,
                [
                    0.580_688_881_036_098_6,
                    0.580_688_881_009_468_8,
                    0.580_688_881_033_406_4,
                    1.0,
                ],
                0.0006,
            ),
        ],
    ),
    (
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::Bt2100Hlg,
        &[
            (
                1,
                [
                    0.436_228_651_804_967_84,
                    0.436_228_651_749_412_8,
                    0.436_228_651_799_355_5,
                    1.0,
                ],
                0.0006,
            ),
            (
                2,
                [
                    0.749_877_365_099_685_5,
                    0.749_877_365_049_653_5,
                    0.749_877_365_094_631_2,
                    1.0,
                ],
                0.0006,
            ),
        ],
    ),
];

/// Each pinned stored value is compared to the oracle's `present_*`
/// value at headroom 2 within the destination's own quantization —
/// ±1 LSB for unorm-8, ~2⁻¹¹ relative for f16 — so a subtle transfer
/// -curve error cannot hide in the broad relative tolerance (#98).
#[test]
fn output_encodings_pin_known_values() -> Result<(), Box<dyn std::error::Error>> {
    let pixels = pixels();
    let (adapter, device, queue) = shared_device()?;
    let source = f16_texture(&device, &queue, &pixels);
    let headroom = 2.0f32;
    for (format, color, anchors) in PINNED {
        let got = present_gpu(
            &adapter,
            &device,
            &queue,
            &source,
            format,
            color,
            OutputAlpha::Premultiplied,
            headroom,
            &pixels,
        )?;
        for (pixel, want, tol) in anchors {
            for (c, (g, w)) in got[*pixel].iter().zip(want.iter()).enumerate() {
                assert!(
                    (g - w).abs() <= *tol,
                    "{color:?} pixel {pixel} channel {c}: shader {g} vs pinned {w}"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn extended_p3_encodes_signed_not_raw_linear() -> Result<(), Box<dyn std::error::Error>> {
    // The ExtendedDisplayP3 surface colour space is *encoded* extended
    // P3: presenting the retained linear-P3 values raw would leave
    // above-one and negative components wrong (#98). Compare against the
    // oracle's encoded extended-P3 presentation — and assert the wire
    // values differ from the linear ones where they must.
    let pixels = pixels();
    let (adapter, device, queue) = shared_device()?;
    let source = f16_texture(&device, &queue, &pixels);
    let headroom = 2.0f32;
    let got = present_gpu(
        &adapter,
        &device,
        &queue,
        &source,
        wgpu::TextureFormat::Rgba16Float,
        OutputColor::ExtendedDisplayP3,
        OutputAlpha::Premultiplied,
        headroom,
        &pixels,
    )?;
    let want = oracle_present(OutputColor::ExtendedDisplayP3, f64::from(headroom), &pixels);
    for (i, (g, w)) in got.iter().zip(&want.pixels).enumerate() {
        assert_close(OutputColor::ExtendedDisplayP3, i, pixels[i], *g, *w);
    }
    // Neutral 2 in extended encoding: sign-symmetric sRGB OETF of 2.0 —
    // strictly between linear 2.0 and 1.0.
    let encoded = got[5][0];
    assert!(
        encoded > 1.0 && encoded < 2.0,
        "extended P3 must encode, not store linear: {encoded}"
    );
    Ok(())
}
