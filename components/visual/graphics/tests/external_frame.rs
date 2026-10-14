//! `ExternalFrameView`'s contract on a real `IOSurface`: decoded planes reach
//! the engine layer without a copy or an intermediate texture, a rebuilt host
//! restarts its source on the new device, and the NV12 and P010 colour paths
//! decode to the colours they encode.
//!
//! The frames are `CVPixelBuffer`s — the storage a video decoder hands out —
//! imported plane by plane through `cherenkov_gpu::interop::metal`, on this
//! machine's Metal device, rendered offscreen.
#![cfg(all(feature = "gpu", target_os = "macos"))]

use std::cell::RefCell;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::rc::Rc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddressOfPlane,
    CVPixelBufferGetBytesPerRowOfPlane, CVPixelBufferGetIOSurface, CVPixelBufferLockBaseAddress,
    CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
    kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange, kCVReturnSuccess,
};
use objc2_metal::{
    MTLDevice, MTLPixelFormat, MTLStorageMode, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use waterui_graphics::cherenkov::{Display, FrameTime, Readback};
use waterui_graphics::cherenkov_gpu::interop::{ExternalFrame, FrameColor, metal};
use waterui_graphics::gpu::{
    ExternalFrameRenderer, ExternalFrameSource, ExternalFrameView, FrameOutput, GpuRuntime,
    RedrawHandle, RetiredOutput,
};
use waterui_graphics::offscreen::{OffscreenImage, OffscreenSize};

/// The frame the colour tests encode: eight bars over a sixteen-step ramp.
const FRAME: (usize, usize) = (256, 128);

/// A source that hands every output it is started with to the test.
struct Producer(Rc<RefCell<Vec<FrameOutput>>>);

impl ExternalFrameSource for Producer {
    fn start(&mut self, output: FrameOutput) {
        self.0.borrow_mut().push(output);
    }

    fn is_opaque(&self) -> bool {
        true
    }
}

fn runtime() -> GpuRuntime {
    pollster::block_on(GpuRuntime::new()).expect("a Metal adapter is required on test hardware")
}

/// A view over a [`Producer`] and the outputs it has been started with.
fn producer_view() -> (ExternalFrameView, Rc<RefCell<Vec<FrameOutput>>>) {
    let outputs = Rc::new(RefCell::new(Vec::new()));
    (
        ExternalFrameView::new(Producer(Rc::clone(&outputs))),
        outputs,
    )
}

fn size(width: usize, height: usize) -> OffscreenSize {
    OffscreenSize::try_from_pixels(
        u32::try_from(width).expect("test sizes fit u32"),
        u32::try_from(height).expect("test sizes fit u32"),
    )
    .expect("test sizes are nonempty")
}

fn renderer(runtime: &GpuRuntime, view: &ExternalFrameView) -> ExternalFrameRenderer {
    ExternalFrameRenderer::new(
        runtime,
        runtime.context(),
        &view.stream(),
        size(FRAME.0, FRAME.1),
        RedrawHandle::new(|| {}),
    )
    .expect("the external frame renderer settles")
}

/// The two YUV layouts a decoder hands out.
#[derive(Clone, Copy)]
enum Layout {
    /// 8-bit 4:2:0 video range.
    Nv12,
    /// 10-bit 4:2:0 video range, codes in the high bits of 16-bit words.
    P010,
}

impl Layout {
    const fn pixel_format(self) -> u32 {
        match self {
            Self::Nv12 => kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            Self::P010 => kCVPixelFormatType_420YpCbCr10BiPlanarVideoRange,
        }
    }

    /// The Metal and wgpu formats of the luma and chroma planes.
    const fn planes(self) -> [(MTLPixelFormat, wgpu::TextureFormat); 2] {
        match self {
            Self::Nv12 => [
                (MTLPixelFormat::R8Uint, wgpu::TextureFormat::R8Uint),
                (MTLPixelFormat::RG8Uint, wgpu::TextureFormat::Rg8Uint),
            ],
            Self::P010 => [
                (MTLPixelFormat::R16Uint, wgpu::TextureFormat::R16Uint),
                (MTLPixelFormat::RG16Uint, wgpu::TextureFormat::Rg16Uint),
            ],
        }
    }
}

/// An `IOSurface`-backed pixel buffer, as `VideoToolbox` produces them.
fn pixel_buffer(layout: Layout, (width, height): (usize, usize)) -> CFRetained<CVPixelBuffer> {
    let surface_properties = CFDictionary::<CFString, CFType>::empty();
    // SAFETY: the key is CoreVideo's own immutable constant.
    let key = unsafe { kCVPixelBufferIOSurfacePropertiesKey };
    let attributes =
        CFDictionary::<CFString, CFType>::from_slices(&[key], &[surface_properties.as_ref()]);
    let mut buffer: *mut CVPixelBuffer = std::ptr::null_mut();
    // SAFETY: the attributes dictionary maps a CFString key to a CFDictionary as
    // CoreVideo documents, and `buffer` is valid storage for the out pointer.
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            width,
            height,
            layout.pixel_format(),
            Some(attributes.as_opaque()),
            NonNull::from(&mut buffer),
        )
    };
    assert_eq!(status, kCVReturnSuccess, "CVPixelBufferCreate failed");
    // SAFETY: a successful create returns a +1 pixel buffer.
    unsafe { CFRetained::from_raw(NonNull::new(buffer).expect("CVPixelBufferCreate succeeded")) }
}

/// Writes one `(y, cb, cr)` code triple per luma pixel through the CPU
/// mapping of the buffer's `IOSurface`. Chroma is taken from the top-left
/// pixel of each 2×2 block.
fn fill(
    buffer: &CVPixelBuffer,
    layout: Layout,
    (width, height): (usize, usize),
    code: impl Fn(usize, usize) -> [u16; 3],
) {
    // SAFETY: the buffer is a live pixel buffer; the lock is paired below.
    let status = unsafe { CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags::empty()) };
    assert_eq!(
        status, kCVReturnSuccess,
        "CVPixelBufferLockBaseAddress failed"
    );
    let luma = CVPixelBufferGetBaseAddressOfPlane(buffer, 0).cast::<u8>();
    let chroma = CVPixelBufferGetBaseAddressOfPlane(buffer, 1).cast::<u8>();
    let luma_stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0);
    let chroma_stride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 1);
    let store = |plane: *mut u8, stride: usize, x: usize, y: usize, value: u16| match layout {
        // SAFETY: `(x, y)` lies inside the plane's rows, which the lock maps.
        Layout::Nv12 => unsafe {
            *plane.add(y * stride + x) = u8::try_from(value).expect("8-bit code");
        },
        // SAFETY: as above; P010 rows hold 16-bit little-endian words with the
        // 10-bit code in the high bits.
        Layout::P010 => unsafe {
            plane
                .add(y * stride + x * 2)
                .cast::<u16>()
                .write_unaligned(value << 6);
        },
    };
    for y in 0..height {
        for x in 0..width {
            let [luma_code, cb, cr] = code(x, y);
            store(luma, luma_stride, x, y, luma_code);
            if x % 2 == 0 && y % 2 == 0 {
                store(chroma, chroma_stride, x, y / 2, cb);
                store(chroma, chroma_stride, x + 1, y / 2, cr);
            }
        }
    }
    // SAFETY: pairs the lock above.
    let status = unsafe { CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags::empty()) };
    assert_eq!(
        status, kCVReturnSuccess,
        "CVPixelBufferUnlockBaseAddress failed"
    );
}

/// The buffer's two planes as `wgpu` textures on `device`, imported in place.
fn import_planes(
    device: &wgpu::Device,
    buffer: &CVPixelBuffer,
    layout: Layout,
    (width, height): (usize, usize),
) -> [wgpu::Texture; 2] {
    let surface = CVPixelBufferGetIOSurface(Some(buffer)).expect("the buffer is IOSurface-backed");
    // SAFETY: the runtime's device is a Metal device on this target.
    let hal = unsafe { device.as_hal::<wgpu::hal::metal::Api>() }.expect("a Metal device");
    let raw_device = hal.raw_device();
    let [luma, chroma] = layout.planes();
    let plane = |index: usize, (mtl, format): (MTLPixelFormat, wgpu::TextureFormat)| {
        let (plane_width, plane_height) = if index == 0 {
            (width, height)
        } else {
            (width.div_ceil(2), height.div_ceil(2))
        };
        // SAFETY: a plain 2D descriptor with positive dimensions.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                mtl,
                plane_width,
                plane_height,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        descriptor.setStorageMode(MTLStorageMode::Shared);
        let texture: Retained<ProtocolObject<dyn MTLTexture>> = raw_device
            .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, index)
            .expect("the IOSurface plane is a Metal texture");
        // SAFETY: the texture was created on the device `device` wraps, and
        // `format` matches its pixel format; the test keeps the pixel buffer
        // alive for as long as frames referencing it render.
        unsafe { metal::import_texture(device, texture, format) }
    };
    [plane(0, luma), plane(1, chroma)]
}

/// A frame over `buffer`'s planes, and a clone of each plane texture.
fn frame(
    device: &wgpu::Device,
    buffer: &CVPixelBuffer,
    layout: Layout,
    dimensions: (usize, usize),
    color: FrameColor,
) -> (ExternalFrame, [wgpu::Texture; 2]) {
    let [y, uv] = import_planes(device, buffer, layout, dimensions);
    let planes = [y.clone(), uv.clone()];
    (
        ExternalFrame::yuv(y, uv, color).expect("the planes meet the frame contract"),
        planes,
    )
}

/// A host texture of `size`, in the float format a native host presents.
fn host_target(device: &wgpu::Device, (width, height): (usize, usize)) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("external frame test target"),
        size: wgpu::Extent3d {
            width: u32::try_from(width).expect("fits"),
            height: u32::try_from(height).expect("fits"),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

const SDR: Display = Display {
    scale: 1.0,
    headroom: 1.0,
};

/// Reads a presented `Rgba16Float` host texture back as sRGB8.
fn read(device: &wgpu::Device, queue: &wgpu::Queue, target: &wgpu::Texture) -> OffscreenImage {
    let (width, height) = (target.width(), target.height());
    let row = width * 8;
    let padded = row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("external frame test readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
    encoder.copy_texture_to_buffer(
        target.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        target.size(),
    );
    queue.submit([encoder.finish()]);
    let (mapped, done) = std::sync::mpsc::channel();
    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
        mapped.send(result).expect("the test waits for the map");
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the readback completes");
    done.recv()
        .expect("the map callback ran")
        .expect("the readback maps");
    let bytes = buffer
        .get_mapped_range(..)
        .expect("the mapped readback is readable");
    let pixels = (0..height)
        .flat_map(|y| {
            let start = (y * padded) as usize;
            bytes[start..start + row as usize]
                .as_chunks::<8>()
                .0
                .iter()
                .map(|texel| {
                    core::array::from_fn(|channel| {
                        half::f16::from_le_bytes([texel[channel * 2], texel[channel * 2 + 1]])
                            .to_f32()
                    })
                })
                .collect::<Vec<[f32; 4]>>()
        })
        .collect();
    OffscreenImage::from_readback(&Readback {
        width,
        height,
        pixels,
    })
}

/// Live `wgpu` textures on the device, from its internal counters.
fn live_textures(device: &wgpu::Device) -> isize {
    device.get_internal_counters().hal.textures.read()
}

/// A flat NV12 frame of one BT.709 video-range colour.
fn flat(code: [u16; 3]) -> impl Fn(usize, usize) -> [u16; 3] {
    move |_, _| code
}

/// BT.709 video-range codes of pure red and pure blue.
const RED: [u16; 3] = [63, 102, 240];
const BLUE: [u16; 3] = [32, 240, 118];

/// The planes are sampled where the decoder wrote them: rewriting the
/// `IOSurface` after the frame was installed changes what the next pass
/// shows, with no new frame published. A copy taken at install would keep
/// showing the old pixels.
#[test]
fn planes_are_sampled_in_place() {
    let runtime = runtime();
    let context = runtime.context();
    let (device, queue) = (context.device(), context.queue());
    let (view, outputs) = producer_view();
    let mut renderer = renderer(&runtime, &view);
    let dimensions = (64, 32);
    let buffer = pixel_buffer(Layout::Nv12, dimensions);
    fill(&buffer, Layout::Nv12, dimensions, flat(RED));
    let (red_frame, _planes) = frame(
        device,
        &buffer,
        Layout::Nv12,
        dimensions,
        FrameColor::BT709_VIDEO,
    );
    outputs.borrow()[0]
        .present(red_frame)
        .expect("the output is live");

    let first = host_target(device, (64, 32));
    renderer
        .present(&first, SDR, FrameTime(std::time::Instant::now()))
        .expect("the frame presents");
    let shown = read(device, queue, &first).pixel(32, 16);
    assert!(
        shown[0] > 200 && shown[2] < 40,
        "the installed frame shows red, got {shown:?}"
    );

    fill(&buffer, Layout::Nv12, dimensions, flat(BLUE));
    // A resize forces the next pass to draw; nothing new was published.
    let second = host_target(device, (96, 48));
    renderer
        .present(&second, SDR, FrameTime(std::time::Instant::now()))
        .expect("the frame presents");
    let shown = read(device, queue, &second).pixel(48, 24);
    assert!(
        shown[2] > 200 && shown[0] < 40,
        "the rewritten IOSurface shows blue without a new frame, got {shown:?}"
    );
}

/// Presenting frames creates no texture of its own: across several frames
/// the device's live textures grow by exactly the planes the test imported.
#[test]
fn frames_add_no_textures_beyond_their_planes() {
    let runtime = runtime();
    let context = runtime.context();
    let (device, queue) = (context.device(), context.queue());
    let (view, outputs) = producer_view();
    let mut renderer = renderer(&runtime, &view);
    let target = host_target(device, (64, 32));
    let dimensions = (64, 32);
    // Every frame's pixel buffer and planes stay alive, so a replaced frame's
    // textures are never destroyed and the count only moves by what the
    // imports and the host create.
    let mut held = Vec::new();
    let mut publish = |code: [u16; 3]| {
        let buffer = pixel_buffer(Layout::Nv12, dimensions);
        fill(&buffer, Layout::Nv12, dimensions, flat(code));
        let (frame, imported) = frame(
            device,
            &buffer,
            Layout::Nv12,
            dimensions,
            FrameColor::BT709_VIDEO,
        );
        outputs.borrow()[0]
            .present(frame)
            .expect("the output is live");
        held.push((buffer, imported));
    };

    // The first frame builds the engine's external-frame pipeline state.
    publish(RED);
    renderer
        .present(&target, SDR, FrameTime(std::time::Instant::now()))
        .expect("the frame presents");
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the frame completes");
    let before = live_textures(device);

    for code in [BLUE, RED, BLUE] {
        publish(code);
        renderer
            .present(&target, SDR, FrameTime(std::time::Instant::now()))
            .expect("the frame presents");
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the frame completes");
    }
    let imported_since = isize::try_from(2 * (held.len() - 1)).expect("a few planes");
    assert_eq!(
        live_textures(device) - before,
        imported_since,
        "only the imported planes may add textures"
    );
    let shown = read(device, queue, &target).pixel(32, 16);
    assert!(
        shown[2] > 200 && shown[0] < 40,
        "the last frame (blue) is on screen, got {shown:?}"
    );
}

/// A host rebuilt on another device — as after a device loss — starts the
/// source again there, and dropping the old host retires only its output,
/// whichever order the two happen in.
#[test]
fn a_rebuilt_host_restarts_the_source_and_retires_the_old_output() {
    let first_runtime = runtime();
    let second_runtime = runtime();
    let (view, outputs) = producer_view();
    let first = renderer(&first_runtime, &view);
    let second = renderer(&second_runtime, &view);
    assert_eq!(outputs.borrow().len(), 2, "each host starts the source");
    assert_eq!(
        outputs.borrow()[1].device(),
        second_runtime.context().device(),
        "the restarted source produces onto the new host's device"
    );

    drop(first);
    let dimensions = (64, 32);
    let buffer = pixel_buffer(Layout::Nv12, dimensions);
    fill(&buffer, Layout::Nv12, dimensions, flat(RED));
    let (stale, _stale_planes) = frame(
        first_runtime.context().device(),
        &buffer,
        Layout::Nv12,
        dimensions,
        FrameColor::BT709_VIDEO,
    );
    assert_eq!(
        outputs.borrow()[0].present(stale),
        Err(RetiredOutput),
        "the dropped host's output refuses frames"
    );
    let (live, _live_planes) = frame(
        second_runtime.context().device(),
        &buffer,
        Layout::Nv12,
        dimensions,
        FrameColor::BT709_VIDEO,
    );
    assert_eq!(
        outputs.borrow()[1].present(live),
        Ok(()),
        "the live host's output still accepts frames"
    );
    drop(second);
    assert!(outputs.borrow()[1].is_retired());
}

/// A renderer is bound to the context it is handed, not to whatever the
/// runtime currently publishes: building one on a context from an older
/// generation reports that generation.
#[test]
fn the_renderer_reports_the_generation_of_the_context_it_was_given() {
    let lost_runtime = runtime();
    let live_runtime = runtime();
    lost_runtime
        .context()
        .mark_device_lost_for_testing("test device loss");
    let fresh = pollster::block_on(lost_runtime.context_after(0));
    assert!(
        fresh.generation() > live_runtime.context().generation(),
        "the rebuild advanced the lost runtime past the live one"
    );
    let (view, _outputs) = producer_view();
    let renderer = ExternalFrameRenderer::new(
        &lost_runtime,
        live_runtime.context(),
        &view.stream(),
        size(FRAME.0, FRAME.1),
        RedrawHandle::new(|| {}),
    )
    .expect("the external frame renderer settles");
    assert_eq!(
        renderer.generation(),
        live_runtime.context().generation(),
        "the renderer is bound to the context it was given"
    );
}

/// A device lost while the host is idle — no frame pending, no display tick
/// — is still recovered: the rebuild publishes through `context_after`, and
/// a renderer built on that fresh context presents on the new device.
#[test]
fn an_idle_device_loss_recovers_through_context_publication() {
    let runtime = runtime();
    let (view, outputs) = producer_view();
    let stale_renderer = renderer(&runtime, &view);
    let stale_generation = stale_renderer.generation();
    runtime
        .context()
        .mark_device_lost_for_testing("test device loss");
    let fresh = pollster::block_on(runtime.context_after(stale_generation));
    assert!(fresh.generation() > stale_generation);
    drop(stale_renderer);

    let device = fresh.device().clone();
    let mut second = ExternalFrameRenderer::new(
        &runtime,
        fresh.clone(),
        &view.stream(),
        size(FRAME.0, FRAME.1),
        RedrawHandle::new(|| {}),
    )
    .expect("the rebuilt external frame renderer settles");
    assert_eq!(second.generation(), fresh.generation());
    let dimensions = (64, 32);
    let buffer = pixel_buffer(Layout::Nv12, dimensions);
    fill(&buffer, Layout::Nv12, dimensions, flat(RED));
    let (red, _planes) = frame(
        &device,
        &buffer,
        Layout::Nv12,
        dimensions,
        FrameColor::BT709_VIDEO,
    );
    outputs.borrow()[1]
        .present(red)
        .expect("the restarted source's output is live");
    let target = host_target(&device, dimensions);
    second
        .present(&target, SDR, FrameTime(std::time::Instant::now()))
        .expect("the frame presents");
    let shown = read(&device, fresh.queue(), &target).pixel(32, 16);
    assert!(
        shown[0] > 200 && shown[2] < 40,
        "the rebuilt host presents the red frame, got {shown:?}"
    );
}

/// The linear-light colours the gallery frames encode, as linear BT.709:
/// eight bars over a sixteen-step grey ramp even in sRGB.
fn target_color(x: usize, y: usize) -> [f64; 3] {
    const BARS: [[f64; 3]; 8] = [
        [1.0, 1.0, 1.0],
        [1.0, 1.0, 0.0],
        [0.0, 1.0, 1.0],
        [0.0, 1.0, 0.0],
        [1.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0],
        [0.0, 0.0, 0.0],
    ];
    if y < FRAME.1 / 2 {
        BARS[x * BARS.len() / FRAME.0]
    } else {
        #[expect(clippy::cast_precision_loss, reason = "a step index is tiny")]
        let encoded = (x * 16 / FRAME.0) as f64 / 15.0;
        let linear = srgb_decode(encoded);
        [linear; 3]
    }
}

fn srgb_decode(encoded: f64) -> f64 {
    if encoded <= 0.040_45 {
        encoded / 12.92
    } else {
        ((encoded + 0.055) / 1.055).powf(2.4)
    }
}

fn srgb_encode(linear: f64) -> f64 {
    if linear <= 0.003_130_8 {
        linear * 12.92
    } else {
        1.055f64.mul_add(linear.powf(1.0 / 2.4), -0.055)
    }
}

/// The BT.709 OETF.
fn bt709_oetf(linear: f64) -> f64 {
    if linear < 0.018 {
        4.5 * linear
    } else {
        1.099f64.mul_add(linear.powf(0.45), -0.099)
    }
}

/// The SMPTE ST 2084 inverse EOTF, from absolute nits.
fn pq_oetf(nits: f64) -> f64 {
    const M1: f64 = 0.159_301_757_812_5;
    const M2: f64 = 78.843_75;
    const C1: f64 = 0.835_937_5;
    const C2: f64 = 18.851_562_5;
    const C3: f64 = 18.687_5;
    let y = (nits / 10_000.0).powf(M1);
    (C2.mul_add(y, C1) / C3.mul_add(y, 1.0)).powf(M2)
}

/// Quantizes encoded `R'G'B'` to video-range `Y'CbCr` codes.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "codes are rounded values inside the video range"
)]
fn video_codes([r, g, b]: [f64; 3], (kr, kb): (f64, f64), bits: u32) -> [u16; 3] {
    let kg = 1.0 - kr - kb;
    let luma = kb.mul_add(b, kr.mul_add(r, kg * g));
    let cb = (b - luma) / (2.0 * (1.0 - kb));
    let cr = (r - luma) / (2.0 * (1.0 - kr));
    let scale = f64::from(1u32 << (bits - 8));
    [
        (219.0 * scale).mul_add(luma, 16.0 * scale).round() as u16,
        (224.0 * scale).mul_add(cb, 128.0 * scale).round() as u16,
        (224.0 * scale).mul_add(cr, 128.0 * scale).round() as u16,
    ]
}

/// NV12 codes for the gallery: BT.709 matrix, BT.709 transfer.
fn nv12_code(x: usize, y: usize) -> [u16; 3] {
    let encoded = target_color(x, y).map(bt709_oetf);
    video_codes(encoded, (0.2126, 0.0722), 8)
}

/// P010 codes for the gallery: the colour moved onto BT.2020 primaries,
/// reference white at 203 nits, PQ-encoded, BT.2020 matrix.
fn p010_code(x: usize, y: usize) -> [u16; 3] {
    const BT709_TO_BT2020: [[f64; 3]; 3] = [
        [0.627_404, 0.329_283, 0.043_313],
        [0.069_097, 0.919_540, 0.011_362],
        [0.016_391, 0.088_013, 0.895_595],
    ];
    let bt709 = target_color(x, y);
    let encoded = BT709_TO_BT2020.map(|row| {
        let linear = row[2].mul_add(bt709[2], row[0].mul_add(bt709[0], row[1] * bt709[1]));
        pq_oetf(203.0 * linear)
    });
    video_codes(encoded, (0.2627, 0.0593), 10)
}

/// The gallery's expected image: the target colours, sRGB-encoded.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "channels are clamped to [0, 255] before the cast"
)]
fn reference() -> OffscreenImage {
    let rgba8 = (0..FRAME.1)
        .flat_map(|y| (0..FRAME.0).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = target_color(x, y).map(|c| (srgb_encode(c) * 255.0).round() as u8);
            [r, g, b, 255]
        })
        .collect();
    OffscreenImage {
        width: u32::try_from(FRAME.0).expect("fits"),
        height: u32::try_from(FRAME.1).expect("fits"),
        rgba8,
    }
}

/// One gallery render: file name, plane layout, code generator, colour.
type GalleryCase = (
    &'static str,
    Layout,
    fn(usize, usize) -> [u16; 3],
    FrameColor,
);

/// Renders the NV12 (BT.709) and P010 (BT.2020 PQ) colour paths from real
/// `IOSurface`s and writes them beside the expected image, for visual review
/// under `$CARGO_TARGET_TMPDIR/external_frame/`.
#[test]
fn gpu_export_external_frame_images() {
    let directory = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("external_frame");
    std::fs::create_dir_all(&directory).expect("the gallery directory is writable");
    reference()
        .save_png(directory.join("reference.png"))
        .expect("the reference encodes");

    let runtime = runtime();
    let context = runtime.context();
    let (device, queue) = (context.device(), context.queue());
    let cases: [GalleryCase; 2] = [
        (
            "nv12_bt709.png",
            Layout::Nv12,
            nv12_code,
            FrameColor::BT709_VIDEO,
        ),
        (
            "p010_bt2020_pq.png",
            Layout::P010,
            p010_code,
            FrameColor::BT2020_PQ,
        ),
    ];
    for (name, layout, code, color) in cases {
        let (view, outputs) = producer_view();
        let mut renderer = renderer(&runtime, &view);
        let buffer = pixel_buffer(layout, FRAME);
        fill(&buffer, layout, FRAME, code);
        let (frame, _planes) = frame(device, &buffer, layout, FRAME, color);
        outputs.borrow()[0]
            .present(frame)
            .expect("the output is live");
        let target = host_target(device, FRAME);
        renderer
            .present(&target, SDR, FrameTime(std::time::Instant::now()))
            .expect("the frame presents");
        read(device, queue, &target)
            .save_png(directory.join(name))
            .expect("the render encodes");
    }
}
