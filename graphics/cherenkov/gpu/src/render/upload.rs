//! Per-frame buffer uploads through a fixed ring of reused staging buffers.
//!
//! `Queue::write_buffer` stages every call in a newly created mappable
//! buffer that lives until its submission completes, so where a frame's
//! upload lands in device memory, and whether it needs a new memory block,
//! depends on how many frames the GPU still holds. The ring allocates its
//! buffers only when a frame's upload first exceeds their capacity. A steady
//! frame reuses the slot the GPU has finished copying out of, and waits for
//! that copy when every slot is still in flight.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use cherenkov::RenderError;

/// Staging buffers in the ring: the CPU uploads at most this many frames
/// ahead of the GPU's completed copies.
pub(super) const SLOTS: usize = 2;

/// The slot's buffer is mapped and writable.
const MAPPED: u8 = 1;
/// The map callback reported an error.
const FAILED: u8 = 2;
/// Written and unmapped; its copies were never submitted.
const WRITTEN: u8 = 3;
/// Submitted; the map is requested and completes after the copy.
const PENDING: u8 = 0;

/// A frame-wide buffer the upload copies into.
#[derive(Clone, Copy, Debug)]
pub(super) enum Dest {
    Instances,
    Stops,
    Globals,
}

/// One region of the slot copied into `dest` at `dst`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Copy {
    pub dest: Dest,
    pub src: u64,
    pub dst: u64,
    pub size: u64,
}

struct Slot {
    buffer: wgpu::Buffer,
    state: Arc<AtomicU8>,
    /// The submission that last copied out of this slot.
    submission: Option<wgpu::SubmissionIndex>,
    #[cfg(target_arch = "wasm32")]
    mapped: Option<futures_channel::oneshot::Receiver<()>>,
}

impl Slot {
    fn new(device: &wgpu::Device, size: u64) -> Self {
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("upload staging"),
                size,
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: true,
            }),
            state: Arc::new(AtomicU8::new(MAPPED)),
            submission: None,
            #[cfg(target_arch = "wasm32")]
            mapped: None,
        }
    }
}

/// What the caller must do before [`Uploads::write`].
pub(super) enum Acquire {
    /// The current slot is mapped.
    Ready,
    /// The current slot's copy is still in flight in this submission; wait
    /// for it, then call [`Uploads::check_mapped`].
    Wait(wgpu::SubmissionIndex),
}

/// The upload ring. Empty until the first frame that uploads.
#[derive(Default)]
pub(super) struct Uploads {
    slots: Vec<Slot>,
    capacity: u64,
    current: usize,
    copies: Vec<Copy>,
}

impl Uploads {
    /// Device bytes the ring holds.
    pub const fn gpu_bytes(&self) -> u64 {
        self.capacity * self.slots.len() as u64
    }

    /// Prepares the current slot for a `size`-byte upload. Grows the ring
    /// when `size` exceeds its capacity; in-flight slots stay alive until
    /// their submissions complete.
    pub fn acquire(&mut self, device: &wgpu::Device, size: u64) -> Result<Acquire, RenderError> {
        self.copies.clear();
        if size > self.capacity {
            let capacity = size.next_power_of_two();
            tracing::debug!(
                from = self.capacity,
                to = capacity,
                slots = SLOTS,
                "upload staging allocated"
            );
            self.slots = (0..SLOTS).map(|_| Slot::new(device, capacity)).collect();
            self.capacity = capacity;
            self.current = 0;
            return Ok(Acquire::Ready);
        }
        let slot = &mut self.slots[self.current];
        match slot.state.load(Ordering::Acquire) {
            MAPPED => Ok(Acquire::Ready),
            // The previous frame failed after writing; its copies never ran.
            WRITTEN => {
                *slot = Slot::new(device, self.capacity);
                Ok(Acquire::Ready)
            }
            FAILED => Err(RenderError::Render("upload staging: the map failed".into())),
            _ => {
                // Completed copies fire their map callbacks here.
                let _ = device.poll(wgpu::PollType::Poll);
                if slot.state.load(Ordering::Acquire) == MAPPED {
                    return Ok(Acquire::Ready);
                }
                let submission = slot
                    .submission
                    .clone()
                    .expect("a pending upload slot was submitted");
                tracing::trace!(slot = self.current, ?submission, "upload slot in flight");
                Ok(Acquire::Wait(submission))
            }
        }
    }

    /// After waiting on [`Acquire::Wait`]'s submission: the slot's map
    /// callback must have run.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn check_mapped(&self) -> Result<(), RenderError> {
        match self.slots[self.current].state.load(Ordering::Acquire) {
            MAPPED => Ok(()),
            FAILED => Err(RenderError::Render("upload staging: the map failed".into())),
            _ => Err(RenderError::Render(
                "upload staging: the map callback did not run after the wait".into(),
            )),
        }
    }

    /// The current slot's map notification, awaited by the browser path.
    #[cfg(target_arch = "wasm32")]
    pub fn take_mapped(&mut self) -> Option<futures_channel::oneshot::Receiver<()>> {
        self.slots[self.current].mapped.take()
    }

    /// The current slot's map outcome once its notification arrived.
    #[cfg(target_arch = "wasm32")]
    pub fn check_mapped(&self) -> Result<(), RenderError> {
        match self.slots[self.current].state.load(Ordering::Acquire) {
            MAPPED => Ok(()),
            _ => Err(RenderError::Render("upload staging: the map failed".into())),
        }
    }

    /// Fills the first `size` bytes of the acquired slot and records the
    /// copies out of it, then unmaps it for the frame's first submission.
    pub fn write(
        &mut self,
        size: u64,
        copies: &[Copy],
        fill: impl FnOnce(&mut wgpu::BufferViewMut),
    ) {
        let slot = &self.slots[self.current];
        {
            let mut view = slot
                .buffer
                .slice(..size)
                .get_mapped_range_mut()
                .expect("buffer range is mapped and not overlapping");
            fill(&mut view);
        }
        slot.buffer.unmap();
        slot.state.store(WRITTEN, Ordering::Release);
        self.copies.clear();
        self.copies.extend_from_slice(copies);
    }

    /// The copies the next submission must encode before its passes, with
    /// the buffer they read. Empty when this frame uploaded nothing or an
    /// earlier surface's encoder already took them.
    pub fn take_copies(&mut self) -> Option<(wgpu::Buffer, Vec<Copy>)> {
        if self.copies.is_empty() {
            return None;
        }
        let copies = std::mem::take(&mut self.copies);
        Some((self.slots[self.current].buffer.clone(), copies))
    }

    /// The submission carrying the copies was queued: request the slot's
    /// map, which completes once the copy has run, and advance the ring.
    pub fn submitted(&mut self, submission: wgpu::SubmissionIndex) {
        let slot = &mut self.slots[self.current];
        slot.submission = Some(submission);
        slot.state.store(PENDING, Ordering::Release);
        let state = Arc::clone(&slot.state);
        #[cfg(target_arch = "wasm32")]
        let (tx, rx) = futures_channel::oneshot::channel();
        #[cfg(target_arch = "wasm32")]
        {
            slot.mapped = Some(rx);
        }
        slot.buffer
            .slice(..)
            .map_async(wgpu::MapMode::Write, move |result| {
                state.store(
                    if result.is_ok() { MAPPED } else { FAILED },
                    Ordering::Release,
                );
                #[cfg(target_arch = "wasm32")]
                let _ = tx.send(());
            });
        self.current = (self.current + 1) % self.slots.len();
    }
}
