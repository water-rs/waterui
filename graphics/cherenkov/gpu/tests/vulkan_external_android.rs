//! Pixel-side `AHardwareBuffer` external-frame verification (issue #166,
//! plan E7). Built in CI for aarch64-linux-android; run the artifact on
//! a Pixel 9 Pro over adb:
//!
//! ```text
//! cargo ndk --target aarch64-linux-android --platform 30 \
//!     build -p cherenkov-gpu --test vulkan_external_android
//! BIN=$(ls -t target/aarch64-linux-android/debug/deps/vulkan_external_android-* | head -1)
//! adb push "$BIN" /data/local/tmp/vulkan_external_android
//! adb shell 'cd /data/local/tmp && LD_LIBRARY_PATH=/data/local/tmp \
//!     ./vulkan_external_android --nocapture --test-threads=1'
//! ```
//!
//! Native `AHardwareBuffer` success is a required result here: the Pixel
//! exposes the AHB, YCbCr and fd-semaphore extensions the import is built
//! on, so a missing capability is a failure, not a skip.
#![cfg(target_os = "android")]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ash::vk;
use ash::vk::Handle as _;
use cherenkov::kurbo::{BezPath, Rect};
use cherenkov::{Draw, Engine, FrameTime, Layer, Next, Surface, WorkingColor};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{
        ExternalFrame, FrameColor, HdrMetadata, OutputAlpha, OutputColor, Presenter, RgbAlpha,
        SharedDevice, TextureOutput, TextureTarget,
        vulkan::{self, FrameSource},
        wgpu,
    },
};

/// The engine device and the native import context — the Pixel must
/// provide every capability the import relies on, so absence fails.
fn setup() -> (SharedDevice, vulkan::Device) {
    let shared = SharedDevice::create(&GpuConfig::default()).expect("shared device");
    assert_eq!(
        shared.adapter.get_info().backend,
        wgpu::Backend::Vulkan,
        "the Pixel test must run on Vulkan"
    );
    let device = vulkan::Device::new(&shared).expect("vulkan import device");
    let caps = device.caps();
    eprintln!("device caps: {caps:?}");
    assert!(caps.timeline_semaphore, "timeline semaphores required");
    assert!(
        caps.sampler_ycbcr_conversion,
        "YCbCr conversion required for the external-format path"
    );
    assert!(caps.external_semaphore_sync_fd, "SYNC_FD export required");
    (shared, device)
}

/// Raw handles the producer side of a test uses.
fn raw(shared: &SharedDevice) -> (ash::Device, vk::Queue, u32) {
    let hal = unsafe { shared.device.as_hal::<wgpu::hal::vulkan::Api>() };
    let hal = hal.as_ref().expect("vulkan device");
    let device = hal.raw_device().clone();
    let queue = hal.raw_queue();
    (device, queue, hal.queue_family_index())
}

/// Allocates an `AHardwareBuffer`; the returned pointer is owned by the
/// caller (one acquire reference).
fn alloc_ahb(
    format: u32,
    width: u32,
    height: u32,
    usage: u64,
) -> Option<*mut ndk_sys::AHardwareBuffer> {
    let desc = ndk_sys::AHardwareBuffer_Desc {
        width,
        height,
        layers: 1,
        format,
        usage,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    if unsafe { ndk_sys::AHardwareBuffer_isSupported(&raw const desc) } == 0 {
        eprintln!("AHB format {format:#x} not supported at {width}x{height}");
        return None;
    }
    let mut buffer = std::ptr::null_mut();
    if unsafe { ndk_sys::AHardwareBuffer_allocate(&raw const desc, &raw mut buffer) } != 0 {
        return None;
    }
    Some(buffer)
}

/// Fills an `RGBA_8888` AHB with `rgba` through a CPU lock — this is the
/// producer writing content, not a consumer copy.
fn make_ahb_rgb(w: u32, h: u32, rgba: [u8; 4]) -> *mut ndk_sys::AHardwareBuffer {
    let usage = ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE.0
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0;
    let buffer = alloc_ahb(
        ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM.0,
        w,
        h,
        usage,
    )
    .expect("RGB AHB");
    let mut addr = std::ptr::null_mut();
    let rc = unsafe {
        ndk_sys::AHardwareBuffer_lock(
            buffer,
            ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0,
            -1,
            std::ptr::null(),
            &raw mut addr,
        )
    };
    assert_eq!(rc, 0, "AHB lock");
    let mut desc = unsafe { std::mem::zeroed::<ndk_sys::AHardwareBuffer_Desc>() };
    unsafe { ndk_sys::AHardwareBuffer_describe(buffer, &raw mut desc) };
    unsafe {
        let stride = desc.stride as usize;
        let base = addr.cast::<u8>();
        for row in 0..h as usize {
            for col in 0..w as usize {
                base.add(row * stride * 4 + col * 4).write_bytes(rgba[0], 1);
                base.add(row * stride * 4 + col * 4 + 1)
                    .write_bytes(rgba[1], 1);
                base.add(row * stride * 4 + col * 4 + 2)
                    .write_bytes(rgba[2], 1);
                base.add(row * stride * 4 + col * 4 + 3)
                    .write_bytes(rgba[3], 1);
            }
        }
        ndk_sys::AHardwareBuffer_unlock(buffer, std::ptr::null_mut());
    }
    buffer
}

/// Fills a `Y8Cb8Cr8_420` AHB with a flat `luma`/`chroma` through
/// `AHardwareBuffer_lockPlanes` — the producer's own content write.
fn make_ahb_nv12(
    width: u32,
    height: u32,
    luma: u8,
    chroma: (u8, u8),
) -> *mut ndk_sys::AHardwareBuffer {
    let usage = ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE.0
        | ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0;
    let buffer = alloc_ahb(
        ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_Y8Cb8Cr8_420.0,
        width,
        height,
        usage,
    )
    .expect("NV12 AHB");
    let mut planes = unsafe { std::mem::zeroed::<ndk_sys::AHardwareBuffer_Planes>() };
    let rc = unsafe {
        ndk_sys::AHardwareBuffer_lockPlanes(
            buffer,
            ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_CPU_WRITE_OFTEN.0,
            -1,
            std::ptr::null(),
            &raw mut planes,
        )
    };
    assert_eq!(rc, 0, "AHB lockPlanes");
    unsafe {
        let luma_plane = planes.planes[0];
        let luma_base = luma_plane.data.cast::<u8>();
        for row in 0..height as usize {
            luma_base
                .add(row * luma_plane.rowStride as usize)
                .write_bytes(luma, width as usize);
        }
        if planes.planeCount >= 3 {
            // Separate U/V planes: fill both.
            let cb = planes.planes[1];
            let cr = planes.planes[2];
            for row in 0..(height as usize) / 2 {
                cb.data
                    .cast::<u8>()
                    .add(row * cb.rowStride as usize)
                    .write_bytes(chroma.0, (width as usize) / 2);
                cr.data
                    .cast::<u8>()
                    .add(row * cr.rowStride as usize)
                    .write_bytes(chroma.1, (width as usize) / 2);
            }
        } else {
            // Interleaved UV: pairs of (U, V).
            let uv = planes.planes[1];
            let uv_base = uv.data.cast::<u8>();
            for row in 0..(height as usize) / 2 {
                for col in 0..(width as usize) / 2 {
                    *uv_base.add(row * uv.rowStride as usize + col * uv.pixelStride as usize) =
                        chroma.0;
                    *uv_base.add(row * uv.rowStride as usize + col * uv.pixelStride as usize + 1) =
                        chroma.1;
                }
            }
        }
        ndk_sys::AHardwareBuffer_unlock(buffer, std::ptr::null_mut());
    }
    buffer
}

/// The work the producer fence submits — freed after its fence signals.
struct ProducerWork {
    /// The pool the delay buffer ran from.
    pool: vk::CommandPool,
    /// The fill destination.
    buffer: vk::Buffer,
    /// Its device-local backing.
    memory: vk::DeviceMemory,
    /// Execution-completion fence for the producer submit.
    fence: vk::Fence,
}

/// A producer sync chain: a binary semaphore whose `SYNC_FD` export is
/// the frame's fence payload. `fire` submits real GPU work — a long run
/// of buffer fills — that signals `binary` when it retires, then exports
/// the pending signal as a `sync_file`.
///
/// `binary` is created with `VkExportSemaphoreCreateInfo` declaring
/// `SYNC_FD`: `VUID-VkSemaphoreGetFdInfoKHR-handleType-01132` makes the
/// export invalid without it, and Mali answers that invalid call with
/// `ERROR_OUT_OF_HOST_MEMORY` — the cause of the earlier failures, not
/// the submission order.
struct ProducerFence {
    /// The sync-file fd the frame waits on, filled by `fire`.
    fd: Mutex<Option<OwnedFd>>,
    /// The timeline semaphore used as the frame's release payload.
    timeline: vk::Semaphore,
    /// The binary semaphore the producer submit signals.
    binary: vk::Semaphore,
    /// The producer submit's objects, freed at drop.
    work: Mutex<Option<ProducerWork>>,
    /// The `VK_KHR_external_semaphore_fd` device-level functions.
    loader: ash::khr::external_semaphore_fd::Device,
    /// The device that owns the semaphores.
    dev: ash::Device,
    /// Instance + physical device for the memory-type query.
    instance: ash::Instance,
    physical: vk::PhysicalDevice,
}

impl ProducerFence {
    /// Arms the fence; `fire` completes it.
    fn new(device: &vulkan::Device) -> Self {
        let dev = device.shared.vk.device.clone();
        let loader = device
            .shared
            .vk
            .external_semaphore_fd
            .clone()
            .expect("sync-fd support checked at setup");
        // The driver must report a binary SYNC_FD semaphore exportable
        // before the export in `fire` can succeed.
        let mut semaphore_type =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::BINARY);
        let info = vk::PhysicalDeviceExternalSemaphoreInfo::default()
            .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD)
            .push_next(&mut semaphore_type);
        let mut external = vk::ExternalSemaphoreProperties::default();
        unsafe {
            device
                .shared
                .instance
                .get_physical_device_external_semaphore_properties(
                    device.shared.physical_device,
                    &info,
                    &mut external,
                );
        };
        assert!(
            external
                .external_semaphore_features
                .contains(vk::ExternalSemaphoreFeatureFlags::EXPORTABLE),
            "binary SYNC_FD semaphores are not exportable on this device: {external:?}"
        );
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let binary = unsafe {
            dev.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                None,
            )
        }
        .expect("binary semaphore");
        let mut type_info =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
        let timeline = unsafe {
            dev.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut type_info),
                None,
            )
        }
        .expect("timeline semaphore");
        Self {
            fd: Mutex::new(None),
            timeline,
            binary,
            work: Mutex::new(None),
            loader,
            dev,
            instance: device.shared.instance.clone(),
            physical: device.shared.physical_device,
        }
    }

    /// The exported sync-file fd; `fire` must have run first.
    fn fd(&self) -> OwnedFd {
        let guard = self.fd.lock().expect("fence fd");
        let fd = guard.as_ref().expect("fence exported by fire");
        let dup = unsafe { OwnedFd::from_raw_fd(libc::dup(fd.as_raw_fd())) };
        drop(guard);
        dup
    }

    /// Submits the producer's delay — `FILLS` 32 MiB fills, real work
    /// handed to the kernel at submit — with `binary` signalled when the
    /// command buffer retires, then exports the pending signal as
    /// `SYNC_FD`. The fence stays unsignalled until the fills finish,
    /// entirely on the GPU.
    fn fire(&self, queue: vk::Queue, family: u32, fills: u32) {
        const FILL_BYTES: u64 = 32 * 1024 * 1024;
        let dev = &self.dev;
        let buffer = unsafe {
            dev.create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(FILL_BYTES)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST)
                    .sharing_mode(vk::SharingMode::EXCLUSIVE),
                None,
            )
        }
        .expect("delay buffer");
        let requirements = unsafe { dev.get_buffer_memory_requirements(buffer) };
        let properties = unsafe {
            self.instance
                .get_physical_device_memory_properties(self.physical)
        };
        let memory_type = (0..properties.memory_type_count)
            .find(|&index| {
                requirements.memory_type_bits & (1 << index) != 0
                    && properties.memory_types[index as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .expect("device-local memory type");
        let memory = unsafe {
            dev.allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(requirements.size)
                    .memory_type_index(memory_type),
                None,
            )
            .expect("delay memory")
        };
        unsafe { dev.bind_buffer_memory(buffer, memory, 0) }.expect("bind delay memory");
        let pool = unsafe {
            dev.create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(family),
                None,
            )
        }
        .expect("pool");
        let cb = unsafe {
            dev.allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .level(vk::CommandBufferLevel::PRIMARY)
                    .command_buffer_count(1),
            )
        }
        .expect("cb")[0];
        let fence = unsafe { dev.create_fence(&vk::FenceCreateInfo::default(), None) }
            .expect("producer fence");
        unsafe {
            dev.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())
                .expect("begin");
            for _ in 0..fills {
                dev.cmd_fill_buffer(cb, buffer, 0, FILL_BYTES, 0xa5);
            }
            dev.end_command_buffer(cb).expect("end");
            dev.queue_submit(
                queue,
                &[vk::SubmitInfo::default()
                    .signal_semaphores(&[self.binary])
                    .command_buffers(&[cb])],
                fence,
            )
            .expect("producer submit");
            // The signal is pending on the GPU now: the SYNC_FD export
            // transplants it into the returned fence.
            let fd = self
                .loader
                .get_semaphore_fd(
                    &vk::SemaphoreGetFdInfoKHR::default()
                        .semaphore(self.binary)
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                )
                .expect("sync-fd export");
            *self.fd.lock().expect("fence fd") = Some(OwnedFd::from_raw_fd(fd));
        }
        *self.work.lock().expect("producer work") = Some(ProducerWork {
            pool,
            buffer,
            memory,
            fence,
        });
    }

    /// Whether the producer's submitted work is still executing — a
    /// non-blocking `vkGetFenceStatus` probe; `fire` must have run first.
    fn pending(&self) -> bool {
        let fence = {
            let work = self.work.lock().expect("producer work");
            work.as_ref().expect("producer submitted").fence
        };
        matches!(unsafe { self.dev.get_fence_status(fence) }, Ok(false))
    }

    /// Blocks until the producer's work retires and returns the wait's
    /// duration — the remaining fill time after `render` returned.
    fn wait_fill(&self) -> Duration {
        let fence = {
            let work = self.work.lock().expect("producer work");
            work.as_ref().expect("producer submitted").fence
        };
        let start = Instant::now();
        unsafe {
            self.dev
                .wait_for_fences(&[fence], true, 30_000_000_000)
                .expect("producer fill wait");
        }
        start.elapsed()
    }
}

impl Drop for ProducerFence {
    fn drop(&mut self) {
        let work = { self.work.lock().expect("producer work").take() };
        if let Some(work) = work {
            unsafe {
                // The delay must retire before its objects are destroyed;
                // a host-side fence wait is the producer's own wait.
                let _ = self
                    .dev
                    .wait_for_fences(&[work.fence], true, 30_000_000_000);
                self.dev.destroy_fence(work.fence, None);
                self.dev.destroy_command_pool(work.pool, None);
                self.dev.destroy_buffer(work.buffer, None);
                self.dev.free_memory(work.memory, None);
            }
        }
        unsafe {
            self.dev.destroy_semaphore(self.timeline, None);
            self.dev.destroy_semaphore(self.binary, None);
        }
    }
}

/// Presents `texture` into a fresh destination and returns its f32
/// pixels (the same helper pattern as `host_contracts`).
fn read_pixels(
    engine: &Engine<Gpu>,
    shared: &SharedDevice,
    output: &wgpu::Texture,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let (target, destinations) = TextureTarget::new((16, 16));
    let destination = engine.surface(target)?;
    let destination_texture = destinations.try_recv()?;
    let delivery = cherenkov_gpu::interop::shader_delivery(wgpu::Backend::Vulkan, &shared.device)?;
    let mut presenter = Presenter::new(&shared.device, delivery);
    presenter.texture(
        &shared.device,
        &shared.queue,
        &output.create_view(&wgpu::TextureViewDescriptor::default()),
        TextureOutput {
            texture: &destination_texture,
            color: OutputColor::LinearDisplayP3,
            alpha: OutputAlpha::Premultiplied,
            headroom: 1.0,
        },
    );
    Ok(destination.readback()?.pixels)
}

/// Every open fd in this process, sorted, as `NUMBER -> TARGET`; the
/// directory fd `read_dir` itself holds is excluded from the result.
fn fd_list() -> Vec<String> {
    let self_dir = format!("/proc/{}/fd", std::process::id());
    let mut list = std::fs::read_dir("/proc/self/fd")
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|entry| {
                    let target = std::fs::read_link(entry.path()).map_or_else(
                        |_| "<unreadable>".to_string(),
                        |path| path.display().to_string(),
                    );
                    (target != self_dir)
                        .then(|| format!("{} -> {target}", entry.file_name().to_string_lossy()))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    list.sort_unstable();
    list
}

/// The entries in `now` not present in `was`, labelled `label`.
fn fd_report(label: &str, was: &[String], now: &[String]) {
    let extra: Vec<&String> = now.iter().filter(|fd| !was.contains(fd)).collect();
    eprintln!("fds {label}: {}", extra.len());
    for fd in extra {
        eprintln!("  {fd}");
    }
}

#[test]
fn ahb_rgb_import_decodes_known_pixels() {
    let (shared, device) = setup();
    let buffer = make_ahb_rgb(16, 16, [0xe0, 0x40, 0x20, 0xff]);
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync: None,
            release: None,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("RGB AHB import");
    eprintln!(
        "RGB frame: size {:?} repr {:?} bytes {}",
        frame.size(),
        frame.repr(),
        frame.imported_bytes()
    );
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let output = textures.try_recv().expect("output texture");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame).expect("external"));
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let pixels = read_pixels(&engine, &shared, &output).expect("readback");
    let pixel = pixels[8 * 16 + 8];
    assert!(
        pixel[0] > 0.5 && pixel[1] < 0.35 && pixel[2] < 0.3 && pixel[3] > 0.99,
        "AHB RGB decode: {pixel:?}"
    );
}

#[test]
fn ahb_yuv_external_format_decodes_neutral() {
    let (shared, device) = setup();
    // Grey content (luma 0x50, neutral chroma) must survive the
    // conversion path to a flat grey — SDR, real external format.
    let buffer = make_ahb_nv12(16, 16, 0x50, (0x80, 0x80));
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync: None,
            release: None,
            color: FrameColor::BT709_VIDEO,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("NV12 AHB import");
    eprintln!(
        "YUV frame: size {:?} repr {:?} bytes {}",
        frame.size(),
        frame.repr(),
        frame.imported_bytes()
    );
    match frame.repr() {
        vulkan::Repr::ExternalFormat { id } => {
            eprintln!("external-format representation, id {id}");
        }
        repr => eprintln!("known-format representation: {repr:?}"),
    }
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let output = textures.try_recv().expect("output texture");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame).expect("external"));
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let pixels = read_pixels(&engine, &shared, &output).expect("readback");
    let pixel = pixels[8 * 16 + 8];
    let max_dev = pixel[..3]
        .iter()
        .fold(0.0f32, |d, c| d.max((c - pixel[0]).abs()));
    assert!(
        pixel[0] > 0.02 && max_dev < 0.08 && pixel[3] > 0.99,
        "YUV neutral chroma decodes to opaque grey: {pixel:?}"
    );
}

/// Imports `buffer` as an external frame with `sync`/`release` and
/// attaches it to `layer`; the AHB acquire reference stays the caller's.
fn ahb_on(
    engine: &Engine<Gpu>,
    device: &vulkan::Device,
    surface: &Surface<Gpu>,
    layer: &Layer,
    buffer: *mut ndk_sys::AHardwareBuffer,
    sync: Option<vulkan::Wait>,
    release: Option<vulkan::ReleaseSync>,
) {
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync,
            release,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("AHB import");
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame).expect("external"));
    surface.update(|tx| {
        tx[layer].content(video.at((16, 16)));
    });
}

/// An engine with a surface and one layer pushed at its root.
fn scene(shared: SharedDevice) -> (Engine<Gpu>, Surface<Gpu>, Layer) {
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, _) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    (engine, surface, layer)
}

/// Renders once and returns the duration.
fn timed_render(engine: &Engine<Gpu>) -> Duration {
    let start = Instant::now();
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    start.elapsed()
}

/// The engine waits on a still-running producer's `sync_file` without
/// blocking the host. `FILLS` 32-MiB fills keep the producer busy well
/// past the render budget; the measured fill time is printed for the
/// device run.
#[test]
fn producer_fence_and_delayed_signal_stay_on_gpu() {
    const FILLS: u32 = 1024;
    let (shared, device) = setup();
    let (_dev, queue, family) = raw(&shared);
    let buffer = make_ahb_rgb(16, 16, [0x20, 0x90, 0x30, 0xff]);

    // Diagnostic, no assertion: is a cold first render the ~450 ms the
    // last device run reported, or a real fence wait? Two fresh engines,
    // the same AHB — one unfenced, one fenced-and-pending — each timed on
    // its first render. A fenced cold render near the unfenced one means
    // cold-start cost; longer by about the fill duration means a real
    // host-side wait in the engine.
    let (engine_u, surface_u, layer_u) = scene(shared.clone());
    ahb_on(&engine_u, &device, &surface_u, &layer_u, buffer, None, None);
    let cold_unfenced = timed_render(&engine_u);

    let fence_cold = ProducerFence::new(&device);
    fence_cold.fire(queue, family, FILLS);
    let (engine, surface, layer) = scene(shared);
    ahb_on(
        &engine,
        &device,
        &surface,
        &layer,
        buffer,
        Some(vulkan::Wait::SyncFd {
            fd: fence_cold.fd(),
        }),
        Some(vulkan::ReleaseSync::Timeline {
            semaphore: fence_cold.timeline.as_raw(),
            value: 2,
        }),
    );
    let cold_fenced = timed_render(&engine);
    eprintln!(
        "cold first render: unfenced {cold_unfenced:?}, fenced {cold_fenced:?} \
         (producer pending: {})",
        fence_cold.pending()
    );

    // The assertion: warm this engine with an unfenced frame through the
    // same surface and layer so every pipeline the fenced render needs
    // exists, then attach a second pending fence and time only that
    // render.
    ahb_on(&engine, &device, &surface, &layer, buffer, None, None);
    timed_render(&engine);

    let fence_warm = ProducerFence::new(&device);
    fence_warm.fire(queue, family, FILLS);
    ahb_on(
        &engine,
        &device,
        &surface,
        &layer,
        buffer,
        Some(vulkan::Wait::SyncFd {
            fd: fence_warm.fd(),
        }),
        Some(vulkan::ReleaseSync::Timeline {
            semaphore: fence_warm.timeline.as_raw(),
            value: 2,
        }),
    );
    let elapsed = timed_render(&engine);
    // The fence must still be pending when `render` returns — otherwise a
    // fast render proves nothing about where the wait ran.
    assert!(
        fence_warm.pending(),
        "producer fills retired before the warmed render returned"
    );
    assert!(
        elapsed < Duration::from_millis(100),
        "submission CPU-waited on the producer fence: {elapsed:?}"
    );
    let remaining = fence_warm.wait_fill();
    eprintln!(
        "warmed fenced render: {elapsed:?}; producer fill needed \
         {remaining:?} more after render returned"
    );
    // The fences' timelines serve as release payloads, so the fences
    // must outlive the engines that signal them.
    drop(engine);
    drop(engine_u);
}

#[test]
fn two_layers_replace_retire_and_release_fence() {
    let (shared, device) = setup();
    let buffer = make_ahb_rgb(16, 16, [0x30, 0x30, 0xa0, 0xff]);
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync: None,
            release: Some(vulkan::ReleaseSync::FenceFd),
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("AHB import");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    textures.try_recv().expect("output texture");
    let (a, b) = (surface.layer(), surface.layer());
    // Two attachments of one generation deduplicate the acquisition.
    let (pa, sa) = engine.frame_producer();
    sa.submit(ExternalFrame::native(frame.clone()).expect("a"));
    let (pb, sb) = engine.frame_producer();
    sb.submit(ExternalFrame::native(frame.clone()).expect("b"));
    surface.update(|tx| {
        tx[surface.root()].push(&a);
        tx[surface.root()].push(&b);
        tx[&a].content(pa.at((16, 16)));
        tx[&b].content(pb.at((16, 16)));
    });
    for _ in 0..3 {
        assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    }
    assert_eq!(frame.generation.lease_count(), 2);
    // Replace one attachment, then drop the other — the last retire
    // submits the release and exports the fence.
    surface.update(|tx| {
        tx[surface.root()].remove(&a);
    });
    drop(a);
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    surface.update(|tx| {
        tx[surface.root()].remove(&b);
    });
    drop(b);
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let fd = frame.release_fd().expect("release fence after submission");
    // The fence signals once the release submission executes; a sync-file
    // fd becomes readable at signal — `poll` is the producer's wait, never
    // the engine's.
    let mut pfd = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let rc = unsafe { libc::poll(&raw mut pfd, 1, 2000) };
    assert_eq!(rc, 1, "release fence did not signal");
}

#[test]
fn native_op_survives_engine_buffer_and_atlas_regrowth() {
    let (shared, device) = setup();
    let buffer = make_ahb_rgb(16, 16, [0xe0, 0x40, 0x20, 0xff]);
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync: None,
            release: None,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("AHB import");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let output = textures.try_recv().expect("output texture");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame).expect("external"));
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    // Regrow the engine's per-draw buffers: enough fills to exceed the
    // initial instance/stops sizing replaces the globals, instances and
    // stops buffers the native op's shared set 0 keys on, so the cached
    // set is rewritten, never allocated a second time.
    let fills = surface.layer();
    surface.update(|tx| {
        // Beneath the external layer: detach, append fills, re-attach.
        tx[surface.root()].remove(&layer);
        tx[surface.root()].push(&fills);
        tx[surface.root()].push(&layer);
        tx[&fills].content(surface.record(|r| {
            for i in 0..40 {
                r.fill(
                    Rect::new(
                        f64::from(i) * 0.2,
                        0.0,
                        f64::from(i).mul_add(0.2, 1.0),
                        16.0,
                    ),
                    WorkingColor::new([0.1, 0.2, 0.3, 1.0]),
                );
            }
        }));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let pixels = read_pixels(&engine, &shared, &output).expect("readback after regrow");
    let pixel = pixels[8 * 16 + 8];
    assert!(
        pixel[0] > 0.5 && pixel[3] > 0.99,
        "native op after buffer regrow: {pixel:?}"
    );
    // Regrow the atlas: fresh path cells past the starting allocation
    // replace the mask texture the per-generation set-1 keys on — the
    // generation-number invalidation must catch the recycled view.
    surface.update(|tx| {
        tx[&fills].content(surface.record(|r| {
            for i in 0..144u32 {
                let mut path = BezPath::new();
                let cx = 0.5 + f64::from(i % 8);
                let cy = 0.5 + f64::from(i / 8);
                for point in 0..5u32 {
                    let angle = f64::from(point)
                        .mul_add(144.0 + f64::from(i), -90.0)
                        .to_radians();
                    let p = (angle.cos().mul_add(0.4, cx), angle.sin().mul_add(0.4, cy));
                    if point == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                path.close_path();
                r.fill(path, WorkingColor::new([0.8, 0.3, 0.1, 1.0]));
            }
        }));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let pixels = read_pixels(&engine, &shared, &output).expect("readback after atlas grow");
    let pixel = pixels[8 * 16 + 8];
    assert!(
        pixel[0] > 0.5 && pixel[3] > 0.99,
        "native op after atlas regrow: {pixel:?}"
    );
}

#[test]
fn next_generation_after_release() {
    let (shared, device) = setup();
    // A reused producer buffer is a new generation with new
    // synchronization, never a mutation of the retired frame.
    let buffer1 = make_ahb_rgb(16, 16, [0xa0, 0x30, 0x30, 0xff]);
    let frame1 = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer1.cast(),
            sync: None,
            release: None,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("gen1");
    let buffer2 = make_ahb_rgb(16, 16, [0x30, 0xa0, 0x30, 0xff]);
    let frame2 = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer2.cast(),
            sync: None,
            release: None,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("gen2");
    assert!(!std::sync::Arc::ptr_eq(
        &frame1.generation,
        &frame2.generation
    ));
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let output = textures.try_recv().expect("output texture");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame2).expect("external"));
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let pixels = read_pixels(&engine, &shared, &output).expect("readback");
    let pixel = pixels[8 * 16 + 8];
    assert!(
        pixel[1] > 0.3 && pixel[0] < 0.3,
        "next generation reads its own content: {pixel:?}"
    );
}

#[test]
fn cancellation_and_teardown() {
    let (shared, device) = setup();
    let buffer = make_ahb_rgb(16, 16, [0x60, 0x60, 0x60, 0xff]);
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync: None,
            release: None,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("AHB import");
    let generation = frame.generation.clone();
    let (dev, _, family) = raw(&shared);
    let pool = unsafe {
        dev.create_command_pool(
            &vk::CommandPoolCreateInfo::default().queue_family_index(family),
            None,
        )
    }
    .expect("pool");
    let cb = unsafe {
        dev.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .expect("cb")[0];
    unsafe {
        dev.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())
            .expect("begin");
    }
    let mut native = vulkan::Native::new(device.shared).expect("native");
    if let Some(pending) = unsafe { vulkan::stage_acquire(&generation, cb) }.expect("stage") {
        native.staged.push(pending);
    }
    vulkan::cancel_staged(&mut native);
    unsafe { dev.destroy_command_pool(pool, None) };
    assert_eq!(
        generation.state(),
        vulkan::State::Registered,
        "a cancelled plan leaves the frame unacquired"
    );
    // Engine teardown with the frame still held must not leak or hang:
    // the engine's final flush retires it.
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, _) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame).expect("external"));
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    drop(engine);
}

#[test]
fn report_counts_and_timings() {
    let (shared, device) = setup();
    let buffer = make_ahb_rgb(16, 16, [0x44, 0x44, 0x44, 0xff]);
    // The engine-lifetime baseline: the device and the test's own AHB
    // reference exist, the engine does not. The closing sample below is
    // taken in the same state — `after == before` means the engine's
    // lifetime leaked nothing.
    let before = fd_list();
    let import_start = Instant::now();
    let frame = device
        .import(FrameSource::Ahb(Box::new(vulkan::Ahb {
            buffer: buffer.cast(),
            sync: None,
            release: None,
            color: FrameColor::SRGB,
            alpha: RgbAlpha::Opaque,
            hdr: HdrMetadata::default(),
        })))
        .expect("AHB import");
    let import_us = import_start.elapsed().as_micros();
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, textures) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    textures.try_recv().expect("output texture");
    let layer = surface.layer();
    let imported = frame.imported_bytes();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame).expect("external"));
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    // Cold vs warm: the first render prepares pipeline state; the second
    // is steady-state.
    let cold = Instant::now();
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    let cold_ms = cold.elapsed().as_secs_f64() * 1e3;
    let mut warm = Vec::new();
    for _ in 0..8 {
        let t = Instant::now();
        assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
        warm.push(t.elapsed().as_secs_f64() * 1e3);
    }
    warm.sort_by(f64::total_cmp);
    let during = fd_list();
    eprintln!("== #166 Pixel report ==");
    eprintln!("import: {import_us}us  bytes: {imported}");
    eprintln!("first render (cold): {cold_ms:.2}ms");
    eprintln!(
        "steady renders ms: p50={:.2} p99={:.2}",
        warm[warm.len() / 2],
        warm[warm.len() - 1]
    );
    eprintln!("fds: before={} during={}", before.len(), during.len());
    fd_report("opened by import + renders", &before, &during);
    eprintln!("memory: {:?}", engine.memory());
    drop(engine);
    // The surface handle is the test's; dropping it releases its
    // retained frame content so the sample measures the engine's whole
    // lifetime against the `before` baseline — the device and the test's
    // AHB reference stay alive in both.
    drop(surface);
    let after = fd_list();
    eprintln!("fds after drop(engine): {}", after.len());
    fd_report("surviving engine teardown", &before, &after);
    assert_eq!(
        after.len(),
        before.len(),
        "leaked fds: {:?}",
        after
            .iter()
            .filter(|fd| !before.contains(fd))
            .collect::<Vec<_>>()
    );
}
