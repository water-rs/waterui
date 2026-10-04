//! Linux dma-buf frames imported through cherenkov-gpu's Vulkan external
//! path — the path the CEF and WPE browser producers take on this
//! platform. The fixture memory is a real dma-buf made from a memfd
//! through `/dev/udmabuf`, so the test needs no GPU card for the
//! descriptor to exist and imports the way any software Vulkan driver
//! would. Hosts without a Vulkan adapter or without udmabuf skip.
#![cfg(all(unix, not(target_vendor = "apple")))]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::interop::vulkan::{self, QueueFamily};
use cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha, SharedDevice, wgpu};
use cherenkov_gpu::{Gpu, GpuConfig};

/// Frame edge in pixels: a 2×2 quadrant grid of 8×8 blocks, matching
/// `external_frames.rs` so the two fixtures review side by side.
const SIZE: u32 = 16;

/// `DRM_FORMAT_ABGR8888` (`drm_fourcc.h`): A in the most significant
/// byte, R in the least — little-endian texel bytes R, G, B, A, which the
/// importer maps to `R8G8B8A8_UNORM`.
const DRM_FORMAT_ABGR8888: u32 = 0x3432_4241;
/// `DRM_FORMAT_MOD_LINEAR` (`drm_fourcc.h`): uncoupled row stride.
const DRM_FORMAT_MOD_LINEAR: u64 = 0;
/// `UDMABUF_CREATE`, `_IOW('u', 0x42, struct udmabuf_create)` — one
/// memfd becomes a dma-buf the kernel can hand to Vulkan. Spelled out so
/// the test needs no ioctl header: WRITE direction, type 'u', nr 0x42.
const UDMABUF_CREATE: u64 =
    (1 << 30) | ((size_of::<UdmabufCreate>() as u64) << 16) | (0x75 << 8) | 0x42;

#[repr(C)]
struct UdmabufCreate {
    memfd: u32,
    flags: u32,
    offset: u64,
    size: u64,
}

/// The quadrant colours as encoded `R'G'B'` (transfer domain):
/// top-left, top-right, bottom-left. The bottom-right quadrant carries a
/// per-column luma ramp, the same fixture `external_frames.rs` renders
/// through wgpu planes — here the bytes live in a dma-buf instead.
const QUADRANTS: [[f64; 3]; 3] = [[0.75, 0.15, 0.15], [0.15, 0.70, 0.20], [0.20, 0.30, 0.85]];

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

/// sRGB EOTF: encoded code back to linear light.
fn srgb_decode(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// sRGB primaries to linear Display P3 — `external_frames.rs`'s
/// `to_linear_p3` made explicit here so this file stands alone.
fn to_linear_p3(rgb: [f64; 3]) -> [f64; 4] {
    let [r, g, b] = rgb;
    [
        0.177_538f64.mul_add(g, 0.822_462 * r),
        0.966_806f64.mul_add(g, 0.033_194 * r),
        0.910_520f64.mul_add(b, 0.072_397f64.mul_add(g, 0.017_083 * r)),
        1.0,
    ]
}

/// A `SharedDevice` on the Vulkan backend plus the native import device,
/// or `None` where no Vulkan adapter exists.
fn setup() -> Option<(SharedDevice, vulkan::Device)> {
    let shared = SharedDevice::create(&GpuConfig::default()).ok()?;
    if shared.adapter.get_info().backend != wgpu::Backend::Vulkan {
        return None;
    }
    let device = vulkan::Device::new(&shared).ok()?;
    let caps = device.caps();
    // A driver without the dma-buf import extensions cannot run this
    // test at all — same skip class as a missing adapter (lavapipe on
    // hosts without a GPU is the common case).
    if !caps.external_memory_dma_buf || !caps.external_memory_fd || !caps.image_drm_format_modifier
    {
        return None;
    }
    Some((shared, device))
}

/// Writes `data` into a fresh memfd and hands it back as a real dma-buf
/// fd, or `None` when the host has no `/dev/udmabuf` — the fixture source
/// the test needs is a kernel feature, not an engine capability.
fn dmabuf(data: &[u8]) -> Result<Option<OwnedFd>, Box<dyn std::error::Error>> {
    // The exported extent must be page-aligned; the fixture's own stride
    // keeps the image inside the first page of the buffer.
    let bytes = data.len().next_multiple_of(4096);
    // SAFETY: `memfd_create` takes no borrowed state; the returned raw fd
    // is wrapped below and errors are checked first.
    let memfd = unsafe {
        libc::memfd_create(
            c"cherenkov-external-frame".as_ptr(),
            // MFD_ALLOW_SEALING: the udmabuf contract below is expressed
            // through seals, so the memfd must be born sealable.
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if memfd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `memfd` is a live, owned descriptor checked just above.
    let memfd = unsafe { OwnedFd::from_raw_fd(memfd) };
    // SAFETY: `memfd` is live and `bytes` is the page-aligned extent.
    if unsafe {
        libc::ftruncate64(
            memfd.as_raw_fd(),
            libc::off64_t::try_from(bytes).expect("extent fits off64_t"),
        )
    } < 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let mut written = 0;
    while written < data.len() {
        // SAFETY: `memfd` is live and `data[written..]` is a valid
        // readable span — `write` copies it out.
        let n = unsafe {
            libc::write(
                memfd.as_raw_fd(),
                data[written..].as_ptr().cast(),
                data.len() - written,
            )
        };
        if n <= 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        written += usize::try_from(n).expect("write length positive");
    }
    // udmabuf pins the memfd's pages for the dma-buf's lifetime; the kernel
    // requires F_SEAL_SHRINK on the memfd and refuses F_SEAL_WRITE.
    // SAFETY: `memfd` is live; F_ADD_SEALS mutates only its seal set.
    if unsafe { libc::fcntl(memfd.as_raw_fd(), libc::F_ADD_SEALS, libc::F_SEAL_SHRINK) } < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `open` takes a path and flags only; the fd is checked below.
    let udmabuf = unsafe { libc::open(c"/dev/udmabuf".as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if udmabuf < 0 {
        // No udmabuf driver (unloaded module or disabled kernel option):
        // same skip class as a missing Vulkan adapter.
        return Ok(None);
    }
    // SAFETY: `udmabuf` is a live, owned descriptor checked just above.
    let udmabuf = unsafe { OwnedFd::from_raw_fd(udmabuf) };
    let create = UdmabufCreate {
        memfd: u32::try_from(memfd.as_raw_fd()).expect("memfd fits u32"),
        // UDMABUF_FLAGS_CLOEXEC — the returned dma-buf fd is close-on-exec
        // like every fd this code path manufactures.
        flags: 0x01,
        offset: 0,
        size: bytes as u64,
    };
    // SAFETY: `udmabuf` is a live udmabuf handle and `create` is a valid
    // `struct udmabuf_create` — the ioctl returns the dma-buf fd.
    let fd = unsafe { libc::ioctl(udmabuf.as_raw_fd(), UDMABUF_CREATE as _, &create) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: `fd` is the dma-buf descriptor the ioctl returned — owned
    // and never used again through its raw number.
    Ok(Some(unsafe { OwnedFd::from_raw_fd(fd) }))
}

/// The encoded fixture bytes, one R8G8B8A8 texel per pixel.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "codes are 8-bit by construction"
)]
fn fixture_bytes() -> Vec<u8> {
    (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            encoded(x, y)
                .map(|c| (c * 255.0).round() as u8)
                .into_iter()
                .chain([255])
                .collect::<Vec<u8>>()
        })
        .collect()
}

#[test]
fn dmabuf_frame_imports_and_decodes_in_place() -> Result<(), Box<dyn std::error::Error>> {
    let Some((shared, device)) = setup() else {
        return Ok(());
    };
    let data = fixture_bytes();
    let Some(fd) = dmabuf(&data)? else {
        return Ok(());
    };
    // The descriptor is exactly what a browser producer builds: one
    // packed linear plane, already-written memory, released on a foreign
    // queue family in GENERAL layout with no GPU fence to wait on.
    let frame = device.import(vulkan::FrameSource::DmaBuf(Box::new(vulkan::DmaBuf {
        fourcc: DRM_FORMAT_ABGR8888,
        modifier: DRM_FORMAT_MOD_LINEAR,
        size: (SIZE, SIZE),
        planes: vec![vulkan::DmaBufPlane {
            memory: 0,
            offset: 0,
            stride: SIZE * 4,
        }],
        memory: vec![fd],
        layout: vulkan::LAYOUT_GENERAL,
        producer_family: QueueFamily::External,
        sync: None,
        release: None,
        color: FrameColor::SRGB,
        alpha: RgbAlpha::Premultiplied,
    })))?;
    assert_eq!(
        frame.repr(),
        vulkan::Repr::Rgb {
            format: wgpu::TextureFormat::Rgba8Unorm,
        },
        "ABGR8888 dma-buf must bind as one Rgba8Unorm plane"
    );

    // The engine runs on the device the frame was imported on — an
    // imported generation installed on another VkDevice is rejected.
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })?;
    let surface = engine.surface(Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16))?;
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame)?);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((SIZE, SIZE)));
    });
    engine.render(FrameTime::now())?;
    let pixels = surface.readback()?.pixels;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let got = pixels[(y * SIZE + x) as usize];
            let [r, g, b] = encoded(x, y);
            let want = to_linear_p3([srgb_decode(r), srgb_decode(g), srgb_decode(b)]);
            for (c, (g_, w_)) in got.iter().zip(want.iter()).enumerate() {
                assert!(
                    (f64::from(*g_) - w_).abs() < 0.01,
                    "dma-buf pixel ({x}, {y}) channel {c}: got {got:?}, expected {want:?}"
                );
            }
        }
    }
    Ok(())
}
