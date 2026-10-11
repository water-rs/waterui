//! The `CVPixelBuffer` pool feeding the video.
//!
//! Each `IOSurface`-backed buffer is filled, wrapped in a Metal texture
//! on the engine's shared device and installed as a new frame
//! generation; a slot is reused only once the surface can no longer be
//! read by anyone. The reuse contract has two halves. Engine side: the
//! texture over a buffer's `IOSurface` is sampled in submissions no
//! later than the render that first draws the next generation — a
//! `on_submitted_work_done` marker past that render means every reader
//! on the queue has finished. Compositor side: the display layer holds
//! a sample until a newer one displays, so a slot must also trail the
//! latest produced generation by a margin before its surface is
//! rewritten.

use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use cherenkov_gpu::interop::wgpu;
use cherenkov_gpu::interop::{ExternalFrame, FrameColor, RgbAlpha, SharedDevice, metal};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::{CFDictionary, CFRetained, CFString, CFType};
use objc2_core_video::{
    CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow,
    CVPixelBufferGetIOSurface, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags,
    CVPixelBufferUnlockBaseAddress, kCVPixelBufferIOSurfacePropertiesKey,
    kCVPixelBufferMetalCompatibilityKey, kCVPixelFormatType_32BGRA, kCVReturnSuccess,
};
use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTextureDescriptor, MTLTextureUsage};

use crate::app::die;
use crate::log;
use crate::pattern::{self, HEIGHT, WIDTH};

/// Four buffers cover the engine's plus the compositor's read latency
/// with slack; a deeper stall simply pauses the video.
const POOL: usize = 4;

/// Generations a retired slot must trail the newest one before its
/// surface is rewritten — one newer sample displayed already replaces it
/// on the plane; the second is margin for the compositor's release.
const RETIRE_MARGIN: u64 = 2;

/// A Metal-compatible, `IOSurface`-backed BGRA pixel buffer.
fn buffer() -> CFRetained<CVPixelBuffer> {
    let empty = CFDictionary::<CFString, CFType>::from_slices(&[], &[]);
    // SAFETY: CoreVideo's attribute keys and the boolean are immutable
    // statics; `out` receives a +1 pixel buffer.
    let attributes = unsafe {
        CFDictionary::<CFString, CFType>::from_slices(
            &[
                kCVPixelBufferIOSurfacePropertiesKey,
                kCVPixelBufferMetalCompatibilityKey,
            ],
            &[
                &empty,
                objc2_core_foundation::kCFBooleanTrue.expect("kCFBooleanTrue"),
            ],
        )
    };
    let mut out = std::ptr::null_mut();
    let status = unsafe {
        CVPixelBufferCreate(
            None,
            WIDTH as usize,
            HEIGHT as usize,
            kCVPixelFormatType_32BGRA,
            Some(attributes.as_opaque()),
            NonNull::from(&mut out),
        )
    };
    assert_eq!(status, kCVReturnSuccess, "CVPixelBufferCreate");
    // SAFETY: the create call returned a +1 pixel buffer.
    unsafe { CFRetained::from_raw(NonNull::new(out).expect("a pixel buffer")) }
}

/// Writes the pattern into `buffer`'s surface.
///
/// # Safety
/// `buffer` is a live BGRA pixel buffer nothing else is writing.
unsafe fn fill(buffer: &CVPixelBuffer, frame: u64) {
    let status = unsafe { CVPixelBufferLockBaseAddress(buffer, CVPixelBufferLockFlags(0)) };
    assert_eq!(status, kCVReturnSuccess, "CVPixelBufferLockBaseAddress");
    let base = CVPixelBufferGetBaseAddress(buffer).cast::<u8>();
    let stride = CVPixelBufferGetBytesPerRow(buffer);
    unsafe { pattern::fill_bgra(base, stride, frame) };
    let status = unsafe { CVPixelBufferUnlockBaseAddress(buffer, CVPixelBufferLockFlags(0)) };
    assert_eq!(status, kCVReturnSuccess, "CVPixelBufferUnlockBaseAddress");
}

/// A buffer's flight state: `None` while the slot is free.
struct Flight {
    /// Its produce index.
    generation: u64,
    /// The render serial whose GPU completion ends all engine reads of
    /// the surface.
    retire_serial: u64,
}

struct Slot {
    buffer: CFRetained<CVPixelBuffer>,
    flight: Option<Flight>,
}

/// A ring of `IOSurface`-backed buffers feeding one video layer.
pub struct Pool {
    device: SharedDevice,
    metal: Retained<ProtocolObject<dyn MTLDevice>>,
    slots: Vec<Slot>,
    /// The launch's `paused` flag: after the first frame is in flight
    /// the pool stops producing, leaving a static video frame on the
    /// layer for the idle-video measurement.
    paused: bool,
    /// Frames handed to the engine.
    pub produced: u64,
    /// Render serials submitted to the engine's queue; a slot's
    /// `retire_serial` names the one after which no submission can still
    /// sample its surface.
    submitted: u64,
    /// Completed serials, stamped by `on_submitted_work_done` callbacks
    /// off the GPU's own completion.
    completed: Arc<AtomicU64>,
    /// Milliseconds the last buffer fill (lock, write, unlock) took.
    pub fill_ms: u64,
    /// Milliseconds the last texture wrap and frame build took.
    pub import_ms: u64,
    /// `produce` calls that found every buffer in flight.
    pub stalls: u64,
    stalled: bool,
}

impl Pool {
    /// Allocates the pool's buffers on the engine's shared device.
    ///
    /// # Panics
    /// When `shared` is not a Metal device.
    #[must_use]
    pub fn new(shared: &SharedDevice, paused: bool) -> Self {
        // SAFETY: the guard is dropped before the device.
        let metal = unsafe { shared.device.as_hal::<wgpu::hal::metal::Api>() }
            .expect("a Metal device")
            .raw_device()
            .clone();
        Self {
            device: shared.clone(),
            metal,
            slots: (0..POOL)
                .map(|_| Slot {
                    buffer: buffer(),
                    flight: None,
                })
                .collect(),
            paused,
            produced: 0,
            submitted: 0,
            completed: Arc::new(AtomicU64::new(0)),
            fill_ms: 0,
            import_ms: 0,
            stalls: 0,
            stalled: false,
        }
    }

    /// Fills a free buffer, wraps it as the next generation and returns
    /// it for installation. `None` while every buffer is in flight, and
    /// once the first frame is out when the pool is `paused`.
    ///
    /// The slot this frame replaces is marked for retirement at the
    /// next render serial — the render that presents its replacement is
    /// the last submission that could still have sampled it.
    pub fn produce(&mut self) -> Option<ExternalFrame> {
        if self.paused && self.produced != 0 {
            return None;
        }
        self.drain();
        let generation = self.produced + 1;
        // The layer's installed slot retires once the render now due —
        // `submitted + 1` — has completed its GPU work.
        if let Some(flight) = self.slots.iter_mut().find_map(|slot| {
            slot.flight
                .as_mut()
                .filter(|flight| flight.generation == self.produced)
        }) {
            flight.retire_serial = self.submitted + 1;
        }
        let Some(free) = self.slots.iter().position(|slot| slot.flight.is_none()) else {
            self.stalls += 1;
            if !self.stalled {
                self.stalled = true;
                log::error("producer stalled: every buffer is still in flight");
            }
            return None;
        };
        self.stalled = false;
        let t = Instant::now();
        unsafe { fill(&self.slots[free].buffer, generation) };
        self.fill_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        let t = Instant::now();
        let frame = self.frame(&self.slots[free].buffer);
        self.import_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        match frame {
            Ok(frame) => {
                self.produced = generation;
                self.slots[free].flight = Some(Flight {
                    generation,
                    retire_serial: u64::MAX,
                });
                Some(frame)
            }
            Err(e) => {
                die(&format!("frame import failed: {e}"));
            }
        }
    }

    /// The `ExternalFrame` over `buffer`'s `IOSurface`: a BGRA texture on
    /// the shared device, declared opaque sRGB — the frame class a plane
    /// can show.
    fn frame(&self, buffer: &CVPixelBuffer) -> Result<ExternalFrame, String> {
        let surface =
            CVPixelBufferGetIOSurface(Some(buffer)).ok_or("the buffer is not IOSurface-backed")?;
        // SAFETY: the descriptor is fully specified.
        let descriptor = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                WIDTH as usize,
                HEIGHT as usize,
                false,
            )
        };
        descriptor.setUsage(MTLTextureUsage::ShaderRead);
        let raw = self
            .metal
            .newTextureWithDescriptor_iosurface_plane(&descriptor, &surface, 0)
            .ok_or("Metal refused an IOSurface texture")?;
        // SAFETY: the texture lives on the engine's device and holds
        // BGRA8 texels; the pool rewrites its surface only after every
        // reader has retired.
        let texture = unsafe {
            metal::import_texture(&self.device.device, raw, wgpu::TextureFormat::Bgra8Unorm)
        };
        ExternalFrame::rgb(texture, RgbAlpha::Opaque, FrameColor::SRGB)
            .map_err(|e| format!("frame contract: {e}"))
    }

    /// Frees slots whose readers finished: `produced - generation`
    /// past [`RETIRE_MARGIN`] (compositor release — the display layer
    /// drops a sample as soon as a newer one shows, so a surface two
    /// generations back is never on screen) and the render serial that
    /// first presented the next generation completed on the GPU
    /// (engine release). `IOSurfaceIsInUse` cannot stand in for the
    /// margin: a buffer's own retention of its surface counts toward
    /// the system-wide use count, so a kept pool buffer never reports
    /// unused.
    fn drain(&mut self) {
        let completed = self.completed.load(Ordering::Acquire);
        for slot in &mut self.slots {
            let Some(flight) = &slot.flight else {
                continue;
            };
            if self.produced >= flight.generation + RETIRE_MARGIN
                && completed >= flight.retire_serial
            {
                slot.flight = None;
            }
        }
    }

    /// After an `Engine::render`: records that a new render serial was
    /// submitted and registers the GPU completion marker for it.
    ///
    /// # Panics
    /// When the shared device is lost while driving completion
    /// callbacks.
    pub fn rendered(&mut self) {
        self.submitted += 1;
        let serial = self.submitted;
        let completed = Arc::clone(&self.completed);
        self.device
            .queue
            .on_submitted_work_done(move || completed.store(serial, Ordering::Release));
        // A plane-only refresh submits nothing to the queue, so the pool
        // drives its own `on_submitted_work_done` callbacks with a poll;
        // an engine-side release signal will replace the margin and the
        // serial (water-rs/cherenkov#254).
        self.device
            .device
            .poll(wgpu::PollType::Poll)
            .expect("the shared device was lost while driving completion callbacks");
    }
}
