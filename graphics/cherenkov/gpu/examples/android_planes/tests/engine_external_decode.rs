//! Host check that the engine-composited path decodes the NV12 pattern:
//! `fill_nv12` writes frame 0 into a two-plane Y+UV mapping, the planes
//! go up as `R8Uint`/`Rg8Uint` textures on the engine's shared device,
//! an `ExternalFrame::yuv` is attached to a layer and rendered to an
//! offscreen surface whose readback is sampled at each quadrant centre.
//!
//! The regression this guards is the engine reading the frame from the
//! wrong origin or reading chroma from the wrong plane — the defect
//! class the Pixel showed as every quadrant of `in-engine` and
//! `no-overlay` sampling the same wrong colour.

use cherenkov::kurbo::Affine;
use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::interop::{ExternalFrame, FrameColor, SharedDevice};
use cherenkov_gpu::{Gpu, GpuConfig};

use android_planes::pattern::{self, HEIGHT, Plane, WIDTH};

/// The quadrant hues the pattern holds, named like `QUADRANTS`:
/// top-left, top-right, bottom-left, bottom-right.
const NAMES: [&str; 4] = ["red", "green", "blue", "gray"];

/// A pixel's hue as seen by dominance: which channel leads, or `None`
/// when all three are equal (the gray quadrant).
fn hue([r, g, b]: [f32; 3]) -> Option<usize> {
    let max = r.max(g).max(b);
    if max - r.min(g).min(b) < 0.02 {
        return None;
    }
    Some([r, g, b].iter().position(|&c| c >= max).unwrap())
}

/// `bytes` as a `TEXTURE_BINDING` texture of `format`, `width` texels
/// and `row_bytes` bytes per row, on the engine's shared queue.
fn upload(
    shared: &SharedDevice,
    bytes: &[u8],
    width: u32,
    row_bytes: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    let height = u32::try_from(bytes.len()).unwrap() / row_bytes;
    let texture = shared.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
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
    shared.queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(row_bytes),
            rows_per_image: Some(height),
        },
        texture.size(),
    );
    texture
}

#[test]
fn the_engine_decodes_the_nv12_pattern() {
    // The exact bytes `fill_nv12` produces for frame 0: tight luma rows,
    // then a tight interleaved UV region.
    let (w, h) = (WIDTH as usize, HEIGHT as usize);
    let mut mapped = vec![0u8; w * h + w * h / 2];
    let (y_bytes, uv_bytes) = mapped.split_at_mut(w * h);
    let planes = [
        Plane {
            data: y_bytes.as_mut_ptr(),
            row_stride: w,
            pixel_stride: 1,
        },
        Plane {
            data: uv_bytes.as_mut_ptr(),
            row_stride: w,
            pixel_stride: 2,
        },
    ];
    unsafe { pattern::fill_nv12(&planes, 0) };

    let shared = SharedDevice::create(&GpuConfig::default()).expect("shared device");
    let (y_bytes, uv_bytes) = mapped.split_at(w * h);
    let y = upload(&shared, y_bytes, WIDTH, WIDTH, wgpu::TextureFormat::R8Uint);
    let uv = upload(
        &shared,
        uv_bytes,
        WIDTH / 2,
        WIDTH,
        wgpu::TextureFormat::Rg8Uint,
    );
    let frame = ExternalFrame::yuv(y, uv, FrameColor::BT709_VIDEO).expect("valid NV12 frame");

    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("host GPU engine");
    let (sw, sh) = (480u32, 270u32);
    let surface = engine
        .surface(Offscreen::new((sw, sh), OffscreenFormat::LinearF16))
        .expect("offscreen surface");
    let video = surface.layer();
    let (prod, sink) = engine.frame_producer();
    sink.submit(frame);
    surface.update(|tx| {
        tx[surface.root()].push(&video);
        // The full frame covers the surface: 1920x1080 scaled by 0.25.
        tx[&video].transform(Affine::scale(0.25));
        tx[&video].content(prod.at((1920, 1080)));
    });
    engine.render(FrameTime::now()).expect("frame renders");
    let read = surface.readback().expect("readable surface");
    assert_eq!((read.width, read.height), (sw, sh));

    let pixel = |x: u32, y: u32| read.pixels[(y * sw + x) as usize];
    // Quadrant centres in the displayed frame.
    let quadrants = [
        pixel(sw / 4, sh / 4),
        pixel(sw * 3 / 4, sh / 4),
        pixel(sw / 4, sh * 3 / 4),
        pixel(sw * 3 / 4, sh * 3 / 4),
    ];
    for (at, [r, g, b, a]) in quadrants.iter().copied().enumerate() {
        assert!(
            a > 0.99,
            "{} quadrant is not opaque: {quadrants:?}",
            NAMES[at]
        );
        assert!(
            max_component(r, g, b) > 0.05,
            "{} quadrant decodes near-black: {quadrants:?}",
            NAMES[at]
        );
    }
    let hues: Vec<Option<usize>> = quadrants
        .iter()
        .map(|&[r, g, b, _]| hue([r, g, b]))
        .collect();
    assert_eq!(hues[0], Some(0), "top-left is not red: {quadrants:?}");
    assert_eq!(hues[1], Some(1), "top-right is not green: {quadrants:?}");
    assert_eq!(hues[2], Some(2), "bottom-left is not blue: {quadrants:?}");
    assert_eq!(hues[3], None, "bottom-right is not gray: {quadrants:?}");
    let distinct: std::collections::HashSet<Option<usize>> = hues.iter().copied().collect();
    assert_eq!(
        distinct.len(),
        4,
        "the quadrants do not decode to four distinct colours: {quadrants:?}"
    );
}

const fn max_component(r: f32, g: f32, b: f32) -> f32 {
    r.max(g).max(b)
}
