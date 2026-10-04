//! External frames decoded in place, with non-uniform content. The
//! fixture — three coloured quadrants and a luma ramp — makes the sample
//! origin, the P010 bit depth and the transfer curve observable; the
//! uniform-colour fixtures the suite had before could catch none of the
//! three. A per-texel f64 reference decode is compared against every
//! rendered pixel, corners included.
//!
//! Each render is written as a PNG under `CHERENKOV_EXTERNAL_PNG_DIR`
//! (default `target/external-frames/`) for visual review.

use std::path::PathBuf;

use cherenkov::{Engine, EngineError, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::interop::{
    ChromaOffset, ExternalFrame, FrameColor, RgbAlpha, SharedDevice, Transfer, wgpu,
};
use cherenkov_gpu::{Gpu, GpuConfig};

/// Frame edge in pixels: a 2×2 quadrant grid of 8×8 blocks.
const SIZE: u32 = 16;

/// The quadrant colours as encoded `R'G'B'` (transfer domain):
/// top-left, top-right, bottom-left. The bottom-right quadrant carries a
/// per-column luma ramp instead of a flat colour.
const QUADRANTS: [[f64; 3]; 3] = [[0.75, 0.15, 0.15], [0.15, 0.70, 0.20], [0.20, 0.30, 0.85]];

/// The encoded `R'G'B'` at luma texel `(x, y)`.
fn encoded(x: u32, y: u32) -> [f64; 3] {
    let half = SIZE / 2;
    match (x >= half, y >= half) {
        (false, false) => QUADRANTS[0],
        (true, false) => QUADRANTS[1],
        (false, true) => QUADRANTS[2],
        (true, true) => {
            let y = f64::from(x - half) / f64::from(half - 1);
            [y, y, y]
        }
    }
}

/// BT.709 `Y'CbCr` of an encoded `R'G'B'` triple, chroma on `[-0.5, 0.5]`.
fn rgbp_to_yuv(rgb: [f64; 3]) -> [f64; 3] {
    let (kr, kb): (f64, f64) = (0.2126, 0.0722);
    let kg = 1.0 - kr - kb;
    let y = kb.mul_add(rgb[2], kr.mul_add(rgb[0], kg * rgb[1]));
    [
        y,
        (rgb[2] - y) / (2.0 * (1.0 - kb)),
        (rgb[0] - y) / (2.0 * (1.0 - kr)),
    ]
}

/// `R'G'B'` of a normalized `Y'CbCr` triple, chroma centred on zero.
fn yuv_to_rgbp(yuv: [f64; 3]) -> [f64; 3] {
    let (kr, kb): (f64, f64) = (0.2126, 0.0722);
    let kg = 1.0 - kr - kb;
    let [y, cb, cr] = yuv;
    [
        2.0f64.mul_add((1.0 - kr) * cr, y),
        (-2.0 * kb * (1.0 - kb) / kg).mul_add(cb, (-2.0 * kr * (1.0 - kr) / kg).mul_add(cr, y)),
        2.0f64.mul_add((1.0 - kb) * cb, y),
    ]
}

/// The video-range code of a normalized component at `bits` depth
/// (luma `16..=235`, chroma `16..=240` at 8 bits, scaled at 10).
fn video_code(component: f64, bits: u32, chroma: bool) -> u32 {
    let (scale, offset) = match (bits, chroma) {
        (8, false) => (219.0, 16.0),
        (8, true) => (224.0, 128.0),
        (10, false) => (876.0, 64.0),
        (10, true) => (896.0, 512.0),
        _ => unreachable!(),
    };
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let code = component.mul_add(scale, offset).round().max(0.0) as u32;
    code
}

/// The normalized component of a video-range code at `bits` depth,
/// chroma centred on zero.
fn video_decode(code: u32, bits: u32, chroma: bool) -> f64 {
    let (scale, offset) = match (bits, chroma) {
        (8, false) => (219.0, 16.0),
        (8, true) => (224.0, 128.0),
        (10, false) => (876.0, 64.0),
        (10, true) => (896.0, 512.0),
        _ => unreachable!(),
    };
    (f64::from(code) - offset) / scale
}

/// The encoded transfer back to display light: the BT.1886 reference EOTF
/// for `Transfer::Bt709` — a pure 2.4 power, black level 0, reference
/// white at 1.0.
fn transfer_decode(c: f64, transfer: Transfer) -> f64 {
    match transfer {
        Transfer::Bt709 => c.max(0.0).powf(2.4),
        Transfer::Linear => c,
        _ => unreachable!("the fixture only uses relative transfers"),
    }
}

/// A decoded signal to premultiplied linear Display P3.
fn to_linear_p3(rgb: [f64; 3]) -> [f64; 4] {
    // The fixture only uses BT.709 primaries.
    let p3 = cherenkov_oracle::color::linear_srgb_to_linear_p3(rgb);
    [p3[0], p3[1], p3[2], 1.0]
}

/// The luma and interleaved-chroma planes of the fixture at `bits` depth,
/// as stored words — P010 carries its 10-bit code in the high bits of the
/// 16-bit word.
fn yuv_fixture(bits: u32) -> (Vec<Vec<u32>>, Vec<Vec<[u32; 2]>>) {
    let half = SIZE / 2;
    let mut y = vec![vec![0u32; SIZE as usize]; SIZE as usize];
    let mut uv = vec![vec![[0u32; 2]; half as usize]; half as usize];
    for row in 0..SIZE {
        for col in 0..SIZE {
            let [ly, cb, cr] = rgbp_to_yuv(encoded(col, row));
            let shift = if bits == 10 { 6 } else { 0 };
            y[row as usize][col as usize] = video_code(ly, bits, false) << shift;
            if col % 2 == 0 && row % 2 == 0 {
                // The fixture's chroma is uniform under every 2×2 texel
                // pair, so the subsampled plane is well defined for any
                // siting.
                uv[(row / 2) as usize][(col / 2) as usize] = [
                    video_code(cb, bits, true) << shift,
                    video_code(cr, bits, true) << shift,
                ];
            }
        }
    }
    (y, uv)
}

/// The expected premultiplied linear-P3 pixel for luma texel `(x, y)` of
/// a YUV fixture at `bits` depth — the fragment stage's own arithmetic:
/// luma at the frame pixel, chroma texel `j` centred on luma
/// `2j + 0.5 + s` (so texel space is `p / 2 + 0.25 - s / 2`), nearest
/// texel (`round(p - 0.5)`), edge-clamped; a P010 word carries its code
/// in the high bits.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "texel indices are small non-negative values"
)]
fn yuv_expected(
    y: &[Vec<u32>],
    uv: &[Vec<[u32; 2]>],
    bits: u32,
    color: &FrameColor,
    x: u32,
    y_px: u32,
) -> [f64; 4] {
    let s = |offset: ChromaOffset| match offset {
        ChromaOffset::Cosited => 0.0,
        ChromaOffset::Centered => 0.5,
    };
    // The shader samples at the pixel centre `x + 0.5`, so chroma texel
    // space is `(x + 0.5) / 2 + 0.25 - s / 2` and the texel is
    // `round(pos - 0.5)` — `x / 2 - s / 2`, ties to even like WGSL `round`.
    let chroma_texel = |p: u32, s: f64, dim: u32| -> u32 {
        s.mul_add(-0.5, f64::from(p) * 0.5)
            .round_ties_even()
            .clamp(0.0, f64::from(dim - 1)) as u32
    };
    let ci = chroma_texel(x, s(color.chroma_siting.x), SIZE / 2);
    let cj = chroma_texel(y_px, s(color.chroma_siting.y), SIZE / 2);
    let shift = if bits == 10 { 64 } else { 1 };
    let yn = video_decode(y[y_px as usize][x as usize] / shift, bits, false);
    let cbn = video_decode(uv[cj as usize][ci as usize][0] / shift, bits, true);
    let crn = video_decode(uv[cj as usize][ci as usize][1] / shift, bits, true);
    let rgbp = yuv_to_rgbp([yn, cbn, crn]);
    to_linear_p3(rgbp.map(|c| transfer_decode(c, color.transfer)))
}

/// The `R'G'B'` code the RGBA8 fixture stores at `(x, y)`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels are clamped to [0,1] before the deliberate u8 quantize"
)]
fn rgb_code(c: f64) -> u8 {
    (c * 255.0).round().clamp(0.0, 255.0) as u8
}

/// The expected premultiplied linear-P3 pixel for texel `(x, y)` of the
/// RGBA8 fixture: the code's unorm value through the transfer decode.
fn rgb_expected(x: u32, y: u32, transfer: Transfer) -> [f64; 4] {
    let lin = encoded(x, y).map(|c| transfer_decode(f64::from(rgb_code(c)) / 255.0, transfer));
    to_linear_p3(lin)
}

/// An adapter/device pair usable as a `SharedDevice` (the engine's fixed
/// shaders are passthrough modules, issue #57).
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

/// An engine plus the device and queue that upload its planes.
type GpuSession = (Engine<Gpu>, wgpu::Device, wgpu::Queue);

/// An engine on the shared device, or `None` where no adapter exists.
fn engine() -> Result<Option<GpuSession>, Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = shared_device()?;
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue: queue.clone(),
        }),
        ..GpuConfig::default()
    });
    match engine {
        Ok(engine) => Ok(Some((engine, device, queue))),
        Err(EngineError::Backend(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// A `TEXTURE_BINDING` plane uploaded from `data` (row-major, `bytes`
/// per texel).
fn plane(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    bytes_per_texel: u32,
    data: &[u8],
) -> wgpu::Texture {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("external test plane"),
        size: wgpu::Extent3d {
            width,
            height,
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
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * bytes_per_texel),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Renders `frame` fullscreen on a `SIZE`² `LinearF16` surface, writes
/// the readback as `name`.png and returns the premultiplied pixels.
fn render(
    engine: &Engine<Gpu>,
    surface: &cherenkov::Surface<Gpu>,
    frame: ExternalFrame,
    name: &str,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(frame);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((SIZE, SIZE)));
    });
    engine.render(FrameTime::now())?;
    let rb = surface.readback()?;
    let dir = std::env::var("CHERENKOV_EXTERNAL_PNG_DIR").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/external-frames"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&dir)?;
    cherenkov_oracle::F32Image {
        width: rb.width,
        height: rb.height,
        pixels: rb.pixels.clone(),
    }
    .write_png(&dir.join(format!("{name}.png")))?;
    Ok(rb.pixels)
}

/// Every pixel must land within the f32-pipeline + f16-storage envelope
/// of the f64 reference; the defects this catches are an order larger.
fn compare(pixels: &[[f32; 4]], expected: impl Fn(u32, u32) -> [f64; 4], what: &str) {
    for y in 0..SIZE {
        for x in 0..SIZE {
            let got = pixels[(y * SIZE + x) as usize];
            let want = expected(x, y);
            for (c, (g, w)) in got.iter().zip(want.iter()).enumerate() {
                assert!(
                    (f64::from(*g) - w).abs() < 0.01,
                    "{what} pixel ({x}, {y}) channel {c}: got {got:?}, expected {want:?}"
                );
            }
        }
    }
}

#[test]
#[expect(
    clippy::cast_possible_truncation,
    reason = "codes are 8-bit by construction"
)]
fn nv12_frame_decodes_in_place() -> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, device, queue)) = engine()? else {
        return Ok(());
    };
    let surface = engine.surface(Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16))?;
    let (luma, uv) = yuv_fixture(8);
    let color = FrameColor::BT709_VIDEO;
    let y_plane = plane(
        &device,
        &queue,
        SIZE,
        SIZE,
        wgpu::TextureFormat::R8Uint,
        1,
        &luma.iter().flatten().map(|c| *c as u8).collect::<Vec<_>>(),
    );
    let uv_plane = plane(
        &device,
        &queue,
        SIZE / 2,
        SIZE / 2,
        wgpu::TextureFormat::Rg8Uint,
        2,
        &uv.iter()
            .flatten()
            .flat_map(|c| [c[0] as u8, c[1] as u8])
            .collect::<Vec<_>>(),
    );
    let frame = ExternalFrame::yuv(y_plane, uv_plane, color)?;
    let pixels = render(&engine, &surface, frame, "nv12")?;
    compare(
        &pixels,
        |x, y| yuv_expected(&luma, &uv, 8, &color, x, y),
        "nv12",
    );
    Ok(())
}

#[test]
#[expect(
    clippy::cast_possible_truncation,
    reason = "codes are 16-bit words by construction"
)]
fn p010_frame_decodes_in_place() -> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, device, queue)) = engine()? else {
        return Ok(());
    };
    let surface = engine.surface(Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16))?;
    let (luma, uv) = yuv_fixture(10);
    let color = FrameColor::BT709_VIDEO;
    let y_plane = plane(
        &device,
        &queue,
        SIZE,
        SIZE,
        wgpu::TextureFormat::R16Uint,
        2,
        &luma
            .iter()
            .flatten()
            .flat_map(|w| (*w as u16).to_le_bytes())
            .collect::<Vec<_>>(),
    );
    let uv_plane = plane(
        &device,
        &queue,
        SIZE / 2,
        SIZE / 2,
        wgpu::TextureFormat::Rg16Uint,
        4,
        &uv.iter()
            .flatten()
            .flat_map(|c| [c[0] as u16, c[1] as u16])
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let frame = ExternalFrame::yuv(y_plane, uv_plane, color)?;
    let pixels = render(&engine, &surface, frame, "p010")?;
    compare(
        &pixels,
        |x, y| yuv_expected(&luma, &uv, 10, &color, x, y),
        "p010",
    );
    Ok(())
}

#[test]
fn rgba_frame_decodes_in_place() -> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, device, queue)) = engine()? else {
        return Ok(());
    };
    let surface = engine.surface(Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16))?;
    let color = FrameColor {
        transfer: Transfer::Bt709,
        ..FrameColor::SRGB
    };
    let data: Vec<u8> = (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = encoded(x, y).map(rgb_code);
            [r, g, b, 255]
        })
        .collect();
    let rgb_plane = plane(
        &device,
        &queue,
        SIZE,
        SIZE,
        wgpu::TextureFormat::Rgba8Unorm,
        4,
        &data,
    );
    let frame = ExternalFrame::rgb(rgb_plane, RgbAlpha::Opaque, color)?;
    let pixels = render(&engine, &surface, frame, "rgba")?;
    compare(&pixels, |x, y| rgb_expected(x, y, color.transfer), "rgba");
    Ok(())
}

/// A solid-colour `SIZE`² RGBA8 frame; `rgba` is sRGB `R'G'B'A'`.
fn solid(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    rgba: [u8; 4],
) -> Result<ExternalFrame, Box<dyn std::error::Error>> {
    let data: Vec<u8> = (0..SIZE * SIZE).flat_map(|_| rgba).collect();
    let plane = plane(
        device,
        queue,
        SIZE,
        SIZE,
        wgpu::TextureFormat::Rgba8Unorm,
        4,
        &data,
    );
    Ok(ExternalFrame::rgb(
        plane,
        RgbAlpha::Opaque,
        FrameColor {
            transfer: Transfer::Linear,
            ..FrameColor::SRGB
        },
    )?)
}

#[test]
fn frame_producer_shows_each_submitted_frame_on_two_surfaces()
-> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, device, queue)) = engine()? else {
        return Ok(());
    };
    let first = engine.surface(Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16))?;
    let second = engine.surface(Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16))?;
    let a = first.layer();
    let b = second.layer();
    let (video, sink) = engine.frame_producer();
    first.update(|tx| {
        tx[first.root()].push(&a);
        tx[&a].content(video.at((SIZE, SIZE)));
    });
    second.update(|tx| {
        tx[second.root()].push(&b);
        tx[&b].content(video.at((SIZE, SIZE)));
    });
    for rgba in [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]] {
        sink.submit(solid(&device, &queue, rgba)?);
        engine.render(FrameTime::now())?;
        let [r, g, b, _] = rgba;
        let expected = to_linear_p3([r, g, b].map(|c| f64::from(c) / 255.0));
        for (name, surface) in [("first", &first), ("second", &second)] {
            compare(&surface.readback()?.pixels, |_, _| expected, name);
        }
    }
    Ok(())
}
