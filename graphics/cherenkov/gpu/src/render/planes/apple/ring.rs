//! A rendered producer's frame ring on Apple: `IOSurface`-backed,
//! scan-out-capable buffers a binding's hardware plane can show
//! directly. The ring is deep enough for the buffers the system
//! compositor may still hold; each buffer is reused only after the
//! compositor released it, and when every buffer is held the frame is
//! skipped with the redraw flag kept — the CPU never blocks.

use super::raster;
use crate::interop::wgpu;
use crate::render::gpu_content::RingBuffer;
use cherenkov::RenderError;

/// How many frames the ring keeps in flight: deep enough for the
/// buffers the system compositor may still hold — a display layer
/// queues several samples before releasing the oldest.
const DEPTH: usize = 4;

/// The Apple ring: `DEPTH` scan-out buffers, rotated while the
/// compositor has released them.
pub struct Ring {
    buffers: Vec<Buffer>,
    /// The next buffer to try.
    next: usize,
    /// The buffers' size; a change reallocates every buffer.
    size: (u32, u32),
    /// Bumped on every reallocation so the producer's slot sees the
    /// planes change.
    version: u64,
}

/// One scan-out buffer and its release baseline.
struct Buffer {
    /// The `IOSurface`-backed, scan-out-capable texture.
    raster: raster::Buffer,
    /// Its sampled view.
    view: wgpu::TextureView,
    /// The surface's retain count while nothing outside the engine
    /// holds it: ours plus the Metal texture's. A queued sample or a
    /// showing plane keeps a further retain on it.
    free: usize,
}

impl Buffer {
    fn new(device: &wgpu::Device, size: (u32, u32), generation: u64) -> Result<Self, RenderError> {
        // Headroom 1.0: display headroom belongs to the static-capture
        // path, not to a producer frame.
        let raster = raster::Buffer::new(device, size, generation, 1.0)?;
        let view = raster
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let free = raster.surface.retain_count();
        Ok(Self { raster, view, free })
    }

    /// Whether something outside the engine still retains the buffer's
    /// `IOSurface` — a sample queued on the display layer, or a showing
    /// plane.
    ///
    /// Retain count, not `IOSurfaceIsInUse`: the system compositor's
    /// hold on a scan-out buffer is a retain, the same shape a
    /// `CVPixelBufferPool` sees when it recycles — `pool_reuse.rs`
    /// (`gpu/examples/apple_planes/tests`) is the reference: while
    /// `IOSurfaceIsInUse` gated the pool, a buffer's own liveness kept
    /// its surface "in use" and the gate could never clear, stalling
    /// every produce past the pool's depth. A retain above the `free`
    /// baseline is the hold that must never be drawn into.
    fn held(&self) -> bool {
        self.raster.surface.retain_count() > self.free
    }
}

/// The buffer index shifted into the low bits of the version the ring
/// reports — a rotation to another buffer changes it even when the
/// buffers were not reallocated.
const fn identity(version: u64, index: usize) -> u64 {
    (version << 8) | index as u64
}

impl Ring {
    /// GPU bytes the ring holds.
    pub fn bytes(&self) -> u64 {
        self.buffers
            .iter()
            .map(|buffer| raster::Buffer::bytes(&buffer.raster))
            .sum()
    }

    /// An empty ring; the first [`next`](Self::next) allocates.
    pub const fn new() -> Self {
        Self {
            buffers: Vec::new(),
            next: 0,
            size: (0, 0),
            version: 0,
        }
    }

    /// The buffer this render draws into, or `None` when every buffer
    /// is still held by the system compositor — the render is skipped
    /// with the redraw flag kept rather than blocking the CPU.
    pub fn next(
        &mut self,
        device: &wgpu::Device,
        size: (u32, u32),
    ) -> Result<Option<RingBuffer<'_>>, RenderError> {
        if self.size != size {
            let mut buffers = Vec::with_capacity(DEPTH);
            for index in 0..DEPTH {
                // `u64::MAX` generation: the ring manages its own frame
                // generations; a buffer's own is unused on this path.
                buffers.push(Buffer::new(device, size, u64::MAX - index as u64)?);
            }
            self.buffers = buffers;
            self.size = size;
            self.next = 0;
            self.version += 1;
        }
        for _ in 0..self.buffers.len() {
            let index = self.next;
            self.next = (self.next + 1) % self.buffers.len();
            let buffer = &self.buffers[index];
            if !buffer.held() {
                return Ok(Some(RingBuffer {
                    texture: &buffer.raster.texture,
                    view: &buffer.view,
                    version: identity(self.version, index),
                }));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> Option<wgpu::Device> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).ok()?;
        Some(device)
    }

    /// The compositor holding every ring buffer skips the frame: `next`
    /// never hands a held buffer to a render, and a released buffer is
    /// the one it returns.
    #[test]
    fn a_held_buffer_is_never_drawn_into() {
        let Some(device) = device() else {
            return;
        };
        let size = (16, 16);
        let mut ring = Ring::new();
        for _ in 0..DEPTH {
            ring.next(&device, size)
                .expect("a buffer")
                .expect("nothing holds a fresh buffer");
        }
        // A retained `IOSurface` is what a queued sample or a showing
        // plane does to a buffer — the hold the ring must never draw into.
        let held: Vec<_> = ring
            .buffers
            .iter()
            .map(|buffer| buffer.raster.surface.clone())
            .collect();
        for _ in 0..DEPTH {
            assert!(
                ring.next(&device, size).expect("the scan").is_none(),
                "a compositor-held buffer is never handed to a render"
            );
        }
        drop(held);
        assert!(
            ring.next(&device, size).expect("the scan").is_some(),
            "a released buffer is drawn into again"
        );
    }
}
