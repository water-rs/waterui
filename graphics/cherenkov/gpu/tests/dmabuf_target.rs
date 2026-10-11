//! The dma-buf render target (#1687): a frame presented into an
//! exported Vulkan image imports back through the existing DMA-BUF path
//! and matches an `Offscreen` render; the bounded pool waits for a
//! release rather than growing; an image's next write is ordered behind
//! the host's release fence on the GPU, never on the CPU.
//!
//! Runs on Vulkan only — a device lacking `VK_EXT_external_memory_dma_buf`
//! or `SYNC_FD` semaphore import/export skips rather than fails.

#![cfg(target_os = "linux")]

use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use ash::vk;

use cherenkov::kurbo::Rect;
use cherenkov::{
    Draw, Engine, EngineError, FrameTime, Offscreen, OffscreenFormat, Surface, WorkingColor,
};
use cherenkov_gpu::interop::dmabuf::{
    DRM_FORMAT_ABGR8888, DRM_FORMAT_ABGR16161616F, DRM_FORMAT_MOD_LINEAR, DmabufFormat,
    DmabufFrame, DmabufTarget,
};
use cherenkov_gpu::interop::vulkan::{
    self, DmaBuf, DmaBufPlane, FrameSource, QueueFamily, ReleaseSync, Wait,
};
use cherenkov_gpu::interop::{ExternalFrame, OutputAlpha, OutputColor, SharedDevice, wgpu};
use cherenkov_gpu::{Gpu, GpuConfig};

const SIZE: u32 = 32;

/// The engine plus its device and the import context.
type Session = (Engine<Gpu>, SharedDevice, vulkan::Device);

/// The engine plus the import context on its device, or `None` where
/// Vulkan or the dma-buf/sync-fd extensions are missing.
fn session() -> Result<Option<Session>, Box<dyn std::error::Error>> {
    let shared = match SharedDevice::create(&GpuConfig::default()) {
        Ok(shared) => shared,
        Err(EngineError::Backend(_)) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    if shared.adapter.get_info().backend != wgpu::Backend::Vulkan {
        eprintln!("dmabuf: adapter is not Vulkan; skipping");
        return Ok(None);
    }
    let import = match vulkan::Device::new(&shared) {
        Ok(import) => import,
        Err(err) => {
            eprintln!("dmabuf: vulkan context failed ({err}); skipping");
            return Ok(None);
        }
    };
    let caps = import.caps();
    if !(caps.external_memory_dma_buf && caps.external_semaphore_sync_fd) {
        eprintln!("dmabuf: no dma-buf export or SYNC_FD semaphores on this driver; skipping");
        return Ok(None);
    }
    let engine = match Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..GpuConfig::default()
    }) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    Ok(Some((engine, shared, import)))
}

/// The raw handles the test's own Vulkan objects live on.
fn raw(shared: &SharedDevice) -> (ash::Instance, ash::Device, vk::Queue, u32) {
    // SAFETY: `session` asserted the adapter is Vulkan; the hal view
    // borrows `shared`, which outlives the returned handles.
    let hal_adapter = unsafe { shared.adapter.as_hal::<wgpu::hal::vulkan::Api>() };
    let instance = hal_adapter
        .expect("vulkan adapter")
        .shared_instance()
        .raw_instance()
        .clone();
    // SAFETY: same, on the device.
    let hal = unsafe { shared.device.as_hal::<wgpu::hal::vulkan::Api>() };
    let hal = hal.as_ref().expect("vulkan device");
    (
        instance,
        hal.raw_device().clone(),
        hal.raw_queue(),
        hal.queue_family_index(),
    )
}

/// A `SYNC_FD`-exportable binary semaphore.
fn exportable_semaphore(device: &ash::Device) -> Result<vk::Semaphore, Box<dyn std::error::Error>> {
    let mut export = vk::ExportSemaphoreCreateInfo::default()
        .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
    let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
    // SAFETY: `device` is live and `info` chains the SYNC_FD export.
    Ok(unsafe { device.create_semaphore(&info, None) }?)
}

/// The semaphore's pending (or completed) signal as a sync file.
fn semaphore_fd(
    instance: &ash::Instance,
    device: &ash::Device,
    semaphore: vk::Semaphore,
) -> Result<OwnedFd, Box<dyn std::error::Error>> {
    let loader = ash::khr::external_semaphore_fd::Device::new(instance, device);
    // SAFETY: `semaphore` is live, declared `SYNC_FD`-exportable and
    // carries a pending signal — the exact window the export requires.
    let fd = unsafe {
        loader.get_semaphore_fd(
            &vk::SemaphoreGetFdInfoKHR::default()
                .semaphore(semaphore)
                .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
        )
    }?;
    // SAFETY: `get_semaphore_fd` returned a new fd owned by the caller.
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// A sync file that signals promptly: a submission signals the
/// semaphore behind it on an idle queue.
fn released_fd(
    instance: &ash::Instance,
    device: &ash::Device,
    queue: vk::Queue,
) -> Result<OwnedFd, Box<dyn std::error::Error>> {
    let semaphore = exportable_semaphore(device)?;
    let submit = vk::SubmitInfo::default().signal_semaphores(std::slice::from_ref(&semaphore));
    // SAFETY: `queue` is live and `semaphore` is its one signal.
    unsafe { device.queue_submit(queue, std::slice::from_ref(&submit), vk::Fence::null()) }?;
    let fd = semaphore_fd(instance, device, semaphore);
    // SAFETY: the export moved the payload to the sync file (or failed
    // and the semaphore is unreferenced) — destroyed exactly once.
    unsafe { device.destroy_semaphore(semaphore, None) };
    fd
}

/// A sync file that stays pending until [`PendingRelease::set`] runs —
/// a host's "consumer not done" release, emulated deterministically by
/// gating the signalling submission on a host event.
struct PendingRelease {
    device: ash::Device,
    queue: vk::Queue,
    event: vk::Event,
    pool: vk::CommandPool,
    fd: OwnedFd,
}

impl PendingRelease {
    /// Queues `signal` behind an unset host event, then exports the
    /// semaphore's pending payload.
    fn new(
        instance: &ash::Instance,
        device: &ash::Device,
        queue: vk::Queue,
        family: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // SAFETY: `device` is live; the event is host-signalled so it
        // is not `DEVICE_ONLY`.
        let event = unsafe { device.create_event(&vk::EventCreateInfo::default(), None) }?;
        let pool_info = vk::CommandPoolCreateInfo::default().queue_family_index(family);
        // SAFETY: `device` is live.
        let pool = unsafe { device.create_command_pool(&pool_info, None) }?;
        let alloc = vk::CommandBufferAllocateInfo::default()
            .command_pool(pool)
            .level(vk::CommandBufferLevel::PRIMARY)
            .command_buffer_count(1);
        // SAFETY: `pool` is live and `alloc` asks for one buffer.
        let cb = unsafe { device.allocate_command_buffers(&alloc) }?[0];
        // SAFETY: `cb` is a fresh primary buffer.
        unsafe {
            device.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())?;
            device.cmd_wait_events(
                cb,
                &[event],
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::BOTTOM_OF_PIPE,
                &[],
                &[],
                &[],
            );
            device.end_command_buffer(cb)?;
        }
        let semaphore = exportable_semaphore(device)?;
        let submit = vk::SubmitInfo::default()
            .command_buffers(std::slice::from_ref(&cb))
            .signal_semaphores(std::slice::from_ref(&semaphore));
        // SAFETY: `queue` is live; the command buffer waits on `event`,
        // keeping `semaphore`'s signal pending.
        unsafe { device.queue_submit(queue, std::slice::from_ref(&submit), vk::Fence::null()) }?;
        let fd = semaphore_fd(instance, device, semaphore)?;
        // SAFETY: the export moved the payload to the sync file.
        unsafe { device.destroy_semaphore(semaphore, None) };
        Ok(Self {
            device: device.clone(),
            queue,
            event,
            pool,
            fd,
        })
    }

    /// Opens the gate: the queued submission completes and the sync file
    /// signals.
    fn set(&self) {
        // SAFETY: `event` is live and host-signalled.
        unsafe { self.device.set_event(self.event) }.expect("set the release gate");
    }
}

impl Drop for PendingRelease {
    fn drop(&mut self) {
        self.set();
        // SAFETY: the gate is open, so the submission drains before its
        // pool dies — `queue_wait_idle` bounds it.
        unsafe {
            let _ = self.device.queue_wait_idle(self.queue);
            self.device.destroy_event(self.event, None);
            self.device.destroy_command_pool(self.pool, None);
        }
    }
}

/// Whether a sync file has signalled, without consuming it.
fn signalled(fd: &OwnedFd, timeout_ms: i32) -> bool {
    let mut pollfd = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: `pollfd` names one live fd; `poll` reports its readiness.
    unsafe { libc::poll(&raw mut pollfd, 1, timeout_ms) == 1 }
}

/// The scene every render of the test draws — two overlapping fills so
/// the output is not a flat colour.
fn draw(surface: &Surface<Gpu>) {
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(4., 4., 28., 28.),
                WorkingColor::new([0.9, 0.2, 0.1, 1.]),
            );
            c.fill(
                Rect::new(12., 12., 20., 20.),
                WorkingColor::new([0.1, 0.5, 0.85, 0.6]),
            );
        }));
    });
}

/// One render plus the frame it must present; a missing frame fails.
fn presented(
    engine: &Engine<Gpu>,
    frames: &Receiver<DmabufFrame>,
) -> Result<DmabufFrame, Box<dyn std::error::Error>> {
    engine.render(FrameTime::now())?;
    Ok(frames.recv_timeout(Duration::from_secs(5))?)
}

/// A target declaring the two export formats every driver this runs on
/// can carry — linear f16 where supported, linear RGBA8 otherwise.
fn target(size: (u32, u32)) -> (DmabufTarget, Receiver<DmabufFrame>) {
    let (target, frames) = DmabufTarget::new(size);
    (
        target.formats([
            DmabufFormat::new(
                DRM_FORMAT_ABGR16161616F,
                vec![DRM_FORMAT_MOD_LINEAR],
                OutputColor::LinearDisplayP3,
                OutputAlpha::Premultiplied,
            ),
            DmabufFormat::new(
                DRM_FORMAT_ABGR8888,
                vec![DRM_FORMAT_MOD_LINEAR],
                OutputColor::Srgb,
                OutputAlpha::Premultiplied,
            ),
        ]),
        frames,
    )
}

/// Re-imports a presented `DmabufFrame` through the engine's own DMA-BUF
/// import — the host's side of the contract, exercised end to end.
fn reimport(
    import: &vulkan::Device,
    family: u32,
    frame: DmabufFrame,
) -> Result<(vulkan::Frame, DmabufFrame), Box<dyn std::error::Error>> {
    let (fds, planes): (Vec<OwnedFd>, Vec<DmaBufPlane>) = frame
        .planes
        .iter()
        .map(|plane| {
            Ok((
                plane.fd.try_clone()?,
                DmaBufPlane {
                    memory: 0,
                    offset: plane.offset,
                    stride: plane.stride,
                },
            ))
        })
        .collect::<Result<_, Box<dyn std::error::Error>>>()
        .map(|pairs: Vec<(OwnedFd, DmaBufPlane)>| pairs.into_iter().unzip())?;
    let imported = import.import(FrameSource::DmaBuf(Box::new(DmaBuf {
        fourcc: frame.fourcc,
        modifier: frame.modifier,
        size: frame.size,
        planes,
        memory: fds,
        layout: frame.layout,
        producer_family: QueueFamily::Index(family),
        sync: Some(Wait::SyncFd {
            fd: frame.acquire.try_clone()?,
        }),
        release: Some(ReleaseSync::FenceFd),
        color: frame.color,
        alpha: frame.alpha,
    })))?;
    Ok((imported, frame))
}

/// Renders `frame` fullscreen on a `SIZE`² `LinearF16` surface and
/// returns the readback pixels.
fn render_frame(
    engine: &Engine<Gpu>,
    surface: &Surface<Gpu>,
    frame: vulkan::Frame,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(ExternalFrame::native(frame)?);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((SIZE, SIZE)));
    });
    engine.render(FrameTime::now())?;
    Ok(surface.readback()?.pixels)
}

#[test]
fn exported_frame_roundtrips_through_import() -> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, shared, import)) = session()? else {
        return Ok(());
    };
    let (instance, device, queue, family) = raw(&shared);
    let (target, frames) = target((SIZE, SIZE));
    let surface = engine.surface(target.pool(1), || {})?;
    draw(&surface);
    let frame = presented(&engine, &frames)?;

    assert_eq!(frame.size, (SIZE, SIZE));
    assert_eq!(frame.modifier, DRM_FORMAT_MOD_LINEAR);
    assert!(
        frame.fourcc == DRM_FORMAT_ABGR16161616F || frame.fourcc == DRM_FORMAT_ABGR8888,
        "negotiated fourcc {:#x} is not a declared one",
        frame.fourcc
    );
    assert_eq!(frame.planes.len(), 1, "a single-plane RGB export");
    let bpp = if frame.fourcc == DRM_FORMAT_ABGR16161616F {
        8
    } else {
        4
    };
    assert!(
        frame.planes[0].stride >= SIZE * bpp,
        "plane stride {} covers a {}-bit row",
        frame.planes[0].stride,
        bpp * 8
    );
    assert!(
        signalled(&frame.acquire, 5000),
        "the acquire fence never signalled"
    );

    let (imported, frame) = reimport(&import, family, frame)?;
    let exported = render_frame(
        &engine,
        &engine.surface(
            Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16),
            || {},
        )?,
        imported.clone(),
    )?;

    let reference = engine.surface(
        Offscreen::new((SIZE, SIZE), OffscreenFormat::LinearF16),
        || {},
    )?;
    draw(&reference);
    engine.render(FrameTime::now())?;
    let expected = reference.readback()?.pixels;

    for (i, (got, want)) in exported.iter().zip(expected.iter()).enumerate() {
        for (c, (g, w)) in got.iter().zip(want.iter()).enumerate() {
            assert!(
                (f64::from(*g) - f64::from(*w)).abs() < 0.02,
                "pixel {i} channel {c}: exported {got:?} vs offscreen {want:?}"
            );
        }
    }

    // The release cycle: the host returns the image with a sync file
    // (here, one that signals promptly). With one image in the pool, the
    // next present proves the slot was freed.
    drop(imported);
    let release = released_fd(&instance, &device, queue)?;
    frame.release(release);
    draw(&surface);
    presented(&engine, &frames)?;
    Ok(())
}

/// A pool with every image out presents nothing until a release
/// arrives — the bounded-pool contract.
#[test]
fn pool_exhaustion_waits_for_release() -> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, shared, _)) = session()? else {
        return Ok(());
    };
    let (instance, device, queue, _) = raw(&shared);
    let (target, frames) = target((SIZE, SIZE));
    let surface = engine.surface(target.pool(2), || {})?;

    draw(&surface);
    let first = presented(&engine, &frames)?;
    draw(&surface);
    let _second = presented(&engine, &frames)?;

    // Both images are out: the next frame presents nothing.
    draw(&surface);
    engine.render(FrameTime::now())?;
    assert!(
        frames.recv_timeout(Duration::from_millis(500)).is_err(),
        "an exhausted pool must not present"
    );

    first.release(released_fd(&instance, &device, queue)?);
    draw(&surface);
    presented(&engine, &frames)?;
    Ok(())
}

/// An image's next write is ordered behind the host's release fence on
/// the GPU: the reused slot's acquire stays pending while the release
/// fence is held.
#[test]
fn release_fence_gates_reuse() -> Result<(), Box<dyn std::error::Error>> {
    let Some((engine, shared, _)) = session()? else {
        return Ok(());
    };
    let (instance, device, queue, family) = raw(&shared);
    let (target, frames) = target((SIZE, SIZE));
    let surface = engine.surface(target.pool(1), || {})?;

    draw(&surface);
    let frame = presented(&engine, &frames)?;
    assert!(
        signalled(&frame.acquire, 5000),
        "the acquire fence never signalled"
    );

    let release = PendingRelease::new(&instance, &device, queue, family)?;
    frame.release(release.fd.try_clone()?);

    draw(&surface);
    let next = presented(&engine, &frames)?;
    assert!(
        !signalled(&next.acquire, 0),
        "the next present must wait on the host's release fence"
    );
    release.set();
    assert!(
        signalled(&next.acquire, 5000),
        "the acquire fence must signal once the release fires"
    );
    Ok(())
}

/// An engine without the import-context gate — negotiation failures
/// stand whether or not the driver exports dma-bufs.
fn engine_only() -> Result<Option<Engine<Gpu>>, Box<dyn std::error::Error>> {
    match Engine::<Gpu>::new(GpuConfig::default()) {
        Ok(engine) => Ok(Some(engine)),
        Err(EngineError::Backend(_)) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// A target declaring nothing the device can export fails fast.
#[test]
fn no_declared_format_fails() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = engine_only()? else {
        return Ok(());
    };
    let (empty, _) = DmabufTarget::new((SIZE, SIZE));
    assert!(matches!(
        engine.surface(empty, || {}),
        Err(cherenkov::SurfaceError::UnsupportedTarget(_))
    ));
    let (bogus, _) = DmabufTarget::new((SIZE, SIZE));
    let bogus = bogus.formats([DmabufFormat::new(
        0xDEAD_BEEF,
        vec![DRM_FORMAT_MOD_LINEAR],
        OutputColor::Srgb,
        OutputAlpha::Opaque,
    )]);
    assert!(matches!(
        engine.surface(bogus, || {}),
        Err(cherenkov::SurfaceError::UnsupportedTarget(_))
    ));
    Ok(())
}
