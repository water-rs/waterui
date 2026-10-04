//! The `AHardwareBuffer` pool: each buffer is filled, imported as a new
//! frame generation, and reused only once its merged release fence
//! (engine + compositor) has signalled.

use std::os::fd::AsRawFd;
use std::os::fd::OwnedFd;
use std::time::Instant;

use ash::vk::{self, Handle as _};
use cherenkov_gpu::interop::RgbAlpha;
use cherenkov_gpu::interop::vulkan::{
    self, Ahb, Frame, FrameSource, NativeError, ReleaseSync, Wait,
};
use ndk_sys::AHardwareBuffer;

use crate::ahb;
use crate::logcat;
use crate::scenario::Spec;

/// Four buffers cover the engine's plus `SurfaceFlinger`'s release latency
/// with slack; a deeper stall simply pauses the video.
const POOL: usize = 4;

/// A ring of `AHardwareBuffer`s feeding one video layer.
pub struct Pool {
    device: vulkan::Device,
    spec: Spec,
    slots: Vec<Slot>,
    /// Frames handed to the engine.
    pub produced: u64,
    /// Release fences observed signalled.
    pub signalled: u64,
    /// The launch's `paused` flag: after the first frame is in flight
    /// the pool stops producing, leaving a static video frame on the
    /// layer for the idle-video measurement.
    paused: bool,
    /// The pre-signalled timeline acquire (`timeline` pools only).
    semaphore: Option<vk::Semaphore>,
    ash: ash::Device,
    stalled: bool,
    /// Milliseconds the last `ahb::fill` (lock, write, unlock) took.
    pub fill_ms: u64,
    /// Milliseconds the last `FrameSource::Ahb` import took.
    pub import_ms: u64,
    /// `produce` calls that found every buffer in flight.
    pub stalls: u64,
}

struct Slot {
    ahb: *mut AHardwareBuffer,
    flight: Option<Flight>,
}

struct Flight {
    /// The live generation; its `release_fd` becomes available once the
    /// release submission runs.
    frame: Frame,
    fd: Option<OwnedFd>,
}

impl Pool {
    /// Allocates the pool's buffers.
    ///
    /// # Errors
    /// When `spec.timeline` is set but the device has no timeline
    /// semaphores, or Vulkan refuses the semaphore.
    pub fn new(device: &vulkan::Device, spec: Spec, paused: bool) -> Result<Self, String> {
        let ash = device.shared.vk.device.clone();
        let semaphore = if spec.timeline {
            if !device.caps().timeline_semaphore {
                return Err("timeline semaphores unsupported".into());
            }
            let mut kind =
                vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
            let semaphore = unsafe {
                ash.create_semaphore(
                    &vk::SemaphoreCreateInfo::default().push_next(&mut kind),
                    None,
                )
            }
            .map_err(|e| format!("timeline semaphore: {e}"))?;
            // Host-signal value 1 once: every frame's acquire at value 1
            // is immediately satisfied.
            unsafe {
                ash.signal_semaphore(
                    &vk::SemaphoreSignalInfo::default()
                        .semaphore(semaphore)
                        .value(1),
                )
            }
            .map_err(|e| format!("timeline signal: {e}"))?;
            Some(semaphore)
        } else {
            None
        };
        let slots = (0..POOL)
            .map(|_| Slot {
                ahb: ahb::alloc(spec.format, spec.overlay),
                flight: None,
            })
            .collect();
        Ok(Self {
            device: device.clone(),
            spec,
            slots,
            paused,
            produced: 0,
            signalled: 0,
            semaphore,
            ash,
            stalled: false,
            fill_ms: 0,
            import_ms: 0,
            stalls: 0,
        })
    }

    /// Fills a free buffer, imports it as the next generation and returns
    /// it for installation. `None` while every buffer is in flight, and
    /// once the first frame is out when the pool is `paused`.
    pub fn produce(&mut self) -> Option<Frame> {
        if self.paused && self.produced != 0 {
            return None;
        }
        self.drain();
        let Some(slot) = self.slots.iter_mut().find(|slot| slot.flight.is_none()) else {
            self.stalls += 1;
            if !self.stalled {
                self.stalled = true;
                logcat::warn("producer stalled: every AHB is still in flight");
            }
            return None;
        };
        self.stalled = false;
        let t = Instant::now();
        unsafe { ahb::fill(self.spec.format, slot.ahb, self.produced) };
        self.fill_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        let sync = self.semaphore.map(|semaphore| Wait::Timeline {
            semaphore: semaphore.as_raw(),
            value: 1,
        });
        let t = Instant::now();
        let frame = self.device.import(FrameSource::Ahb(Box::new(Ahb {
            buffer: slot.ahb.cast(),
            sync,
            release: Some(ReleaseSync::FenceFd),
            color: self.spec.color,
            alpha: RgbAlpha::Opaque,
            hdr: self.spec.hdr,
        })));
        self.import_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        match frame {
            Ok(frame) => {
                self.produced += 1;
                slot.flight = Some(Flight {
                    frame: frame.clone(),
                    fd: None,
                });
                Some(frame)
            }
            Err(e) => {
                logcat::error(&format!("frame import failed: {e}"));
                None
            }
        }
    }

    /// Counts signalled release fences and frees their buffers.
    fn drain(&mut self) {
        for slot in &mut self.slots {
            let Some(flight) = &mut slot.flight else {
                continue;
            };
            if flight.fd.is_none() {
                match flight.frame.release_fd() {
                    Ok(fd) => flight.fd = Some(fd),
                    Err(NativeError::Unready) => continue,
                    Err(e) => {
                        logcat::warn(&format!("release_fd failed: {e}"));
                        continue;
                    }
                }
            }
            let Some(fd) = &flight.fd else {
                continue;
            };
            let mut pfd = libc::pollfd {
                fd: fd.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let rc = unsafe { libc::poll(&raw mut pfd, 1, 0) };
            if rc > 0 {
                slot.flight = None;
                self.signalled += 1;
            }
        }
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        for slot in &self.slots {
            unsafe { ahb::release(slot.ahb) };
        }
        if let Some(semaphore) = self.semaphore {
            unsafe { self.ash.destroy_semaphore(semaphore, None) };
        }
    }
}
