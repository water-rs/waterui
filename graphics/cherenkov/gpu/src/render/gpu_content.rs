//! Retained GPU producers, shared across surfaces.
//!
//! A producer's output is an [`ExternalFrame`]: a rendered producer draws
//! into a buffer from its renderer-owned frame ring — what the ring is
//! belongs to the surface's compositor contract, one wgpu texture where
//! there are no system planes — and that buffer becomes the producer's
//! current frame, which every binding samples as
//! [`ImageSource::Content`](super::lower::ImageSource::Content). A
//! submitted-frame producer's current frame is whatever its
//! [`FrameSink`](cherenkov::FrameSink) last installed. Allocation and
//! setup happen the first frame a binding is drawn — a producer with no
//! drawn binding renders nothing.

use crate::interop::{
    ExternalFrame, FrameColor, FramePlanes, GpuContentBox, RgbAlpha,
    wgpu::{Context, Frame},
};
use cherenkov::{Instant, ProducerId, RenderError, SurfaceVisibility};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::planes::Compositor;

/// A [`GpuProducer`](cherenkov::GpuProducer)'s renderer state: the frame
/// ring it draws into, its current frame and its wake state. One
/// producer serves bindings on every surface of its engine; the surfaces
/// that draw it each frame drive its wake gate.
pub struct Producer {
    /// The rendered producer's content; `None` for a submitted-frame
    /// producer, whose frames come from
    /// [`FrameSink::submit`](cherenkov::FrameSink::submit).
    content: Option<GpuContentBox>,
    /// The shared dirty flag: the content's redraw flag for a rendered
    /// producer, the sink's submit flag for a frame producer.
    dirty: Arc<AtomicBool>,
    /// The wake gate the surfaces drawing the producer's bindings drive.
    gate: Arc<cherenkov::WakeGate>,
    /// The rendered producer's frame ring; `None` for a frame producer.
    ring: Option<Ring>,
    /// The ring instance's identity — bumped every time the ring is
    /// recreated, so a buffer identity from one ring can never collide
    /// with the next ring's.
    ring_epoch: u64,
    /// The ring buffers' size — the componentwise largest size the
    /// frame's drawn bindings requested. A change reallocates the ring
    /// without setup.
    size: (u32, u32),
    /// The producer's current frame and its decode slot — the
    /// `ExternalFrame` every binding samples.
    current: Option<super::external::Slot>,
    /// Whether the platform's compositor can show the current frame on a
    /// hardware plane.
    on_plane: bool,
    /// The current frame's hand-off generation, bumped on every install:
    /// a plane showing a binding hands the system a new buffer when it
    /// changes.
    generation: u64,
    /// Which ring buffer `current`'s planes live in — the identity the
    /// ring reports; a rotation to another buffer changes it, so the
    /// slot's views and binds rebuild only when the planes change.
    buffer: u64,
    initialized: bool,
    origin: Option<Instant>,
    last_frame: Option<Instant>,
    last_scale: Option<f32>,
    again: bool,
}

impl Drop for Producer {
    fn drop(&mut self) {
        self.gate.close();
    }
}

/// A layer's binding to a producer, at the pixel size the layer needs.
/// Bindings are installed, replaced and dropped only through the
/// transaction stream — a size change is a new binding.
pub struct Binding {
    /// The producer handle: the binding keeps the producer alive until
    /// it is released, and its [`ProducerId`] names the
    /// [`super::lower::ImageSource::Content`] the binding samples.
    producer: cherenkov::GpuProducer<crate::Gpu>,
    /// The output size this binding requests.
    pub size: (u32, u32),
}

impl Binding {
    pub const fn new(producer: cherenkov::GpuProducer<crate::Gpu>, size: (u32, u32)) -> Self {
        Self { producer, size }
    }

    /// The producer this binding samples.
    pub fn producer(&self) -> ProducerId {
        self.producer.id()
    }
}

impl Producer {
    /// A rendered producer: draws into its frame ring, one render per
    /// frame at most.
    pub fn rendered(content: GpuContentBox) -> Self {
        Self {
            dirty: Arc::clone(&content.redraw.dirty),
            gate: Arc::clone(&content.redraw.gate),
            content: Some(content),
            ring: None,
            ring_epoch: 0,
            size: (0, 0),
            current: None,
            on_plane: false,
            generation: 0,
            buffer: u64::MAX,
            initialized: false,
            origin: None,
            last_frame: None,
            last_scale: None,
            again: false,
        }
    }

    /// A submitted-frame producer: `dirty` and `gate` are the
    /// [`FrameSink`](cherenkov::FrameSink)'s shared wake state — a submit
    /// marks the producer dirty and wakes while the gate is open.
    pub const fn submitted(dirty: Arc<AtomicBool>, gate: Arc<cherenkov::WakeGate>) -> Self {
        Self {
            content: None,
            dirty,
            gate,
            ring: None,
            ring_epoch: 0,
            size: (0, 0),
            current: None,
            on_plane: false,
            generation: 0,
            buffer: u64::MAX,
            initialized: false,
            origin: None,
            last_frame: None,
            last_scale: None,
            again: false,
        }
    }

    /// The producer's current frame — the one every binding samples —
    /// `None` before a rendered producer draws and before a frame
    /// producer's first submit.
    pub const fn current(&self) -> Option<&super::external::Slot> {
        self.current.as_ref()
    }

    /// The producer's current frame, mutably — the encode path binds it.
    pub const fn current_mut(&mut self) -> Option<&mut super::external::Slot> {
        self.current.as_mut()
    }

    /// GPU bytes the producer holds: its ring plus its current frame's
    /// slot. The ring's buffers count once each, however many bindings
    /// show them.
    pub fn gpu_bytes(&self) -> u64 {
        self.ring.as_ref().map_or(0, Ring::bytes)
            + u64::from(self.current.is_some()) * super::external::Slot::GPU_BYTES
    }

    /// Hands the rendered producer's content to a device replacement:
    /// the next renderer registers it again and its first drawn binding
    /// runs `setup` on the new device. A submitted-frame producer has no
    /// content — its frame dropped with the device and the sink's next
    /// submit supplies one on the new device.
    pub fn into_content(mut self) -> Option<GpuContentBox> {
        self.content.take()
    }

    /// The surfaces that drew the producer this frame: a redraw request
    /// or a submission wakes the host while one of them is announced
    /// visible — the frame-membership model `update_filter_activity`
    /// applies to filters.
    pub fn set_gate(&self, surfaces: &[SurfaceVisibility]) {
        self.gate.set(surfaces);
    }

    pub fn wants_redraw(&self) -> bool {
        // A rendered producer without a current frame has never drawn;
        // a frame producer waiting on its first submit is not stale.
        (self.content.is_some() && self.current.is_none())
            || self.again
            || self.dirty.load(Ordering::Acquire)
    }

    /// A submission's frame becomes the producer's current frame — the
    /// submitted planes are new buffers every time, so the slot is
    /// rebuilt. A frame producer has no setup: the frame arrived on the
    /// device that will draw it.
    pub fn submit(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, frame: ExternalFrame) {
        self.generation += 1;
        self.on_plane = super::planes::Platform::shows(&frame);
        self.current = Some(super::external::Slot::new(
            device,
            queue,
            frame,
            self.generation,
            self.on_plane,
        ));
        // The submission's planes are not the ring's: the next rendered
        // or submitted frame rebuilds the slot rather than reusing views.
        self.buffer = u64::MAX;
    }

    /// The drawn ring buffer becomes the producer's current frame: its
    /// planes are the ring's, so when the buffer is the same as last
    /// frame's — one texture where there are no system planes — the
    /// slot keeps its views, params and binds and only the generation
    /// moves.
    fn set_current_frame(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: ExternalFrame,
        buffer: u64,
    ) {
        self.generation += 1;
        self.on_plane = super::planes::Platform::shows(&frame);
        match &mut self.current {
            Some(slot) if buffer == self.buffer => slot.swap_frame(frame, self.generation),
            _ => {
                self.buffer = buffer;
                self.current = Some(super::external::Slot::new(
                    device,
                    queue,
                    frame,
                    self.generation,
                    self.on_plane,
                ));
            }
        }
    }

    /// The frame every binding samples: the ring buffer the producer
    /// drew into, in the working space, decoding as the identity.
    const fn ring_frame(texture: wgpu::Texture) -> ExternalFrame {
        ExternalFrame {
            planes: FramePlanes::Rgb {
                plane: texture,
                alpha: RgbAlpha::Premultiplied,
            },
            color: FrameColor::LINEAR_P3,
            wait: None,
        }
    }

    /// Draws into the next ring buffer for `size` at `scale` — the
    /// largest attachment and scale this frame's drawn bindings asked
    /// across every surface — and makes that buffer the producer's
    /// current frame. Runs setup the first time it draws, reallocates
    /// the ring on a size change without setup, and runs at most once
    /// per frame. When every ring buffer is compositor-held the frame is
    /// skipped with the redraw flag kept: the CPU never blocks.
    ///
    /// A submitted-frame producer draws nothing: its flag is consumed
    /// here, the submitted frame having landed through
    /// [`submit`](Self::submit).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        time: Instant,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(), RenderError> {
        if self.content.is_none() {
            // A submitted-frame producer draws nothing: its flag is
            // consumed, the submitted frame having landed through
            // [`submit`](Self::submit).
            self.dirty.swap(false, Ordering::AcqRel);
            return Ok(());
        }
        let maximum = device.limits().max_texture_dimension_2d;
        if size.0 == 0 || size.1 == 0 || size.0 > maximum || size.1 > maximum {
            return Err(RenderError::Render(format!(
                "GPU producer size {size:?} must be nonzero and at most {maximum}"
            )));
        }
        if self.size != size {
            // A size change reallocates the ring without setup and draws
            // into the new attachment; the current frame stays until the
            // next draw replaces it.
            self.size = size;
            self.ring = None;
            self.ring_epoch += 1;
        } else if !self.wants_redraw() && self.last_scale.map(f32::to_bits) == Some(scale.to_bits())
        {
            return Ok(());
        }
        // Consume before setup/render, so an asynchronous request during either
        // remains pending and schedules another frame.
        self.dirty.swap(false, Ordering::AcqRel);
        if !self.initialized {
            let content = self.content.as_mut().expect("checked above");
            let redraw = content.redraw.clone();
            content.content.setup(&Context {
                adapter,
                device,
                queue,
                format: super::TARGET_FORMAT,
                redraw,
            });
            self.initialized = true;
        }
        let next = self.ring.get_or_insert_with(Ring::new).next(device, size);
        #[cfg(target_vendor = "apple")]
        let next = next?;
        #[cfg(not(target_vendor = "apple"))]
        let next = Some(next);
        let Some(buffer) = next else {
            // Every ring buffer is held by a compositor: skip to the next
            // frame with the redraw flag kept — the CPU never blocks.
            self.again = true;
            return Ok(());
        };
        let mut frame = Frame {
            device,
            queue,
            texture: buffer.texture,
            view: buffer.view,
            format: super::TARGET_FORMAT,
            width: self.size.0,
            height: self.size.1,
            scale,
            elapsed: time.saturating_duration_since(*self.origin.get_or_insert(time)),
            delta: self
                .last_frame
                .map_or(Duration::ZERO, |last| time.saturating_duration_since(last)),
            redraw: false,
        };
        self.content
            .as_mut()
            .expect("checked above")
            .content
            .render(&mut frame);
        self.again = frame.redraw;
        self.last_frame = Some(time);
        self.last_scale = Some(scale);
        let texture = buffer.texture.clone();
        let identity = (self.ring_epoch << 32) | buffer.version;
        self.set_current_frame(device, queue, Self::ring_frame(texture), identity);
        Ok(())
    }

    /// [`render`](Self::render), on the browser executor.
    #[cfg(target_arch = "wasm32")]
    #[expect(
        clippy::future_not_send,
        reason = "the browser engine is single-threaded and its futures run on the page's event loop"
    )]
    pub async fn render(
        &mut self,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        time: Instant,
        size: (u32, u32),
        scale: f32,
    ) -> Result<(), RenderError> {
        if self.content.is_none() {
            self.dirty.swap(false, Ordering::AcqRel);
            return Ok(());
        }
        let maximum = device.limits().max_texture_dimension_2d;
        if size.0 == 0 || size.1 == 0 || size.0 > maximum || size.1 > maximum {
            return Err(RenderError::Render(format!(
                "GPU producer size {size:?} must be nonzero and at most {maximum}"
            )));
        }
        if self.size != size {
            self.size = size;
            self.ring = None;
            self.ring_epoch += 1;
        } else if !self.wants_redraw() && self.last_scale.map(f32::to_bits) == Some(scale.to_bits())
        {
            return Ok(());
        }
        self.dirty.swap(false, Ordering::AcqRel);
        if !self.initialized {
            let content = self.content.as_mut().expect("checked above");
            let redraw = content.redraw.clone();
            content
                .content
                .setup(&Context {
                    adapter,
                    device,
                    queue,
                    format: super::TARGET_FORMAT,
                    redraw,
                })
                .await;
            self.initialized = true;
        }
        let next = self.ring.get_or_insert_with(Ring::new).next(device, size);
        #[cfg(target_vendor = "apple")]
        let next = next?;
        #[cfg(not(target_vendor = "apple"))]
        let next = Some(next);
        let Some(buffer) = next else {
            self.again = true;
            return Ok(());
        };
        let mut frame = Frame {
            device,
            queue,
            texture: buffer.texture,
            view: buffer.view,
            format: super::TARGET_FORMAT,
            width: self.size.0,
            height: self.size.1,
            scale,
            elapsed: time.saturating_duration_since(*self.origin.get_or_insert(time)),
            delta: self
                .last_frame
                .map_or(Duration::ZERO, |last| time.saturating_duration_since(last)),
            redraw: false,
        };
        self.content
            .as_mut()
            .expect("checked above")
            .content
            .render(&mut frame);
        self.again = frame.redraw;
        self.last_frame = Some(time);
        self.last_scale = Some(scale);
        let texture = buffer.texture.clone();
        let identity = (self.ring_epoch << 32) | buffer.version;
        self.set_current_frame(device, queue, Self::ring_frame(texture), identity);
        Ok(())
    }
}

/// The ring buffer a render draws into.
pub struct RingBuffer<'a> {
    /// The buffer's texture.
    pub texture: &'a wgpu::Texture,
    /// Its sampled view.
    pub view: &'a wgpu::TextureView,
    /// The buffer's identity within the ring — a rotation or a
    /// reallocation changes it, so the producer's slot rebuilds only
    /// when the planes do.
    pub version: u64,
}

/// A rendered producer's frame ring.
///
/// Where the platform has no system planes the ring is exactly one wgpu
/// texture: queue order serializes the render before every read, so
/// nothing outside the engine ever holds it. On Apple the ring is
/// `IOSurface`-backed, scan-out-capable buffers deep enough for the
/// buffers the system compositor may still hold, each reused only after
/// the compositor released it
/// ([`planes::apple::ring`](super::planes::apple::ring)).
#[cfg(not(target_vendor = "apple"))]
pub struct Ring {
    image: Option<super::GpuImage>,
    /// Bumped on every reallocation so the slot sees the texture change.
    version: u64,
}

#[cfg(target_vendor = "apple")]
pub use super::planes::apple::ring::Ring;

#[cfg(not(target_vendor = "apple"))]
impl Ring {
    /// An empty ring; the first [`next`](Self::next) allocates.
    const fn new() -> Self {
        Self {
            image: None,
            version: 0,
        }
    }

    /// GPU bytes the ring holds.
    fn bytes(&self) -> u64 {
        self.image.as_ref().map_or(0, |image| {
            u64::from(image.width)
                * u64::from(image.height)
                * super::texel_bytes(super::TARGET_FORMAT)
        })
    }

    fn allocate(device: &wgpu::Device, size: (u32, u32)) -> super::GpuImage {
        let (texture, view) = super::create_target(
            device,
            "GPU content frame",
            size,
            super::TARGET_USAGES,
            super::TARGET_FORMAT,
        );
        crate::diag::create(
            device,
            "GPU content frame",
            u64::from(size.0) * u64::from(size.1) * super::texel_bytes(super::TARGET_FORMAT),
        );
        super::GpuImage {
            texture,
            view,
            width: size.0,
            height: size.1,
        }
    }

    /// The buffer this render draws into: the one buffer is freed by
    /// queue order alone, so it is never held when `next` is called.
    fn next(&mut self, device: &wgpu::Device, size: (u32, u32)) -> RingBuffer<'_> {
        if self.image.as_ref().map(|i| (i.width, i.height)) != Some(size) {
            self.image = Some(Self::allocate(device, size));
            self.version += 1;
        }
        let image = self.image.as_ref().expect("allocated above");
        RingBuffer {
            texture: &image.texture,
            view: &image.view,
            version: self.version,
        }
    }
}
