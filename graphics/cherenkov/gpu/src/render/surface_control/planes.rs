//! One engine surface realized as a stack of child surface controls.
//!
//! The stack, bottom to top, interleaves the engine's own composited parts
//! with promoted external frames. Every frame the engine blits each part
//! into one of that part's plane buffers in a single submission, exports
//! the submission's completion as a sync fence, and applies one
//! transaction that sets every part's buffer, every promoted frame's buffer
//! and every property that changed — so a promoted frame and the content
//! around it change on the same display frame.
//!
//! The transaction's completion hands back the release fence of every
//! buffer it replaced or removed: the engine's own buffers return to their
//! part's pool behind that fence, and a promoted frame's fence joins the
//! producer's release fence before the plane's lease on the frame ends.

use std::os::fd::OwnedFd;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use ash::vk;
use cherenkov::kurbo::Affine;
use cherenkov::{LayerId, RenderError, ShapeData, SurfaceError};
use rustc_hash::FxHashMap;

use super::buffer::{self, Buffer, State};
use super::ffi::{SurfaceControl, Transaction};
use super::plan::{
    self, BufferTransform, Dataspace, Encoding, Geometry, IRect, Inexpressible, Op, Placement,
    Properties, Slot,
};
use crate::interop::android::SurfaceControlTarget;
use crate::interop::{
    ExternalFrame, FramePlanes, HdrMetadata, OutputAlpha, OutputColor, TextureOutput,
};
use crate::render::external::vulkan::{self, NativeError, PlaneAcquire, ReleaseSync, Repr, Shared};
use crate::render::planes::{Composition, Compositor, PlaneContent, SystemPlanes};
use crate::render::present::Presenter;

/// Buffers per engine part: one shown, one queued, one being drawn.
const BUFFERS_PER_PART: usize = 3;

/// A promoted external frame's place in the stack.
struct Promotion<'a> {
    /// The layer the frame is the content of.
    layer: LayerId,
    /// The frame.
    frame: &'a vulkan::Frame,
    /// The surface's install count when the frame was installed.
    generation: u64,
    /// Everything the transaction sets on its plane but the z-order.
    properties: Properties,
}

/// One entry of a surface's plane stack, bottom to top.
enum Entry<'a> {
    /// Engine-composited content: premultiplied linear Display P3.
    Engine {
        view: Option<&'a wgpu::TextureView>,
        size: (u32, u32),
        raster: Option<(LayerId, u64)>,
        properties: Properties,
    },
    /// A frame shown on its own plane.
    Frame(Promotion<'a>),
}

/// Why a frame's buffer cannot be shown on an Android plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum Ineligible {
    /// The frame is not an `AHardwareBuffer`.
    #[error("the frame is not an AHardwareBuffer")]
    NotABuffer,
    /// The buffer was not allocated for hardware overlays.
    #[error("the buffer lacks AHARDWAREBUFFER_USAGE_COMPOSER_OVERLAY")]
    NoOverlayUsage,
    /// The producer's acquire is a Vulkan semaphore, which the system
    /// compositor cannot wait on.
    #[error("the frame's acquire is a Vulkan semaphore, not a sync fence")]
    SemaphoreAcquire,
    /// The producer's release is a timeline semaphore, which a plane's
    /// release fence cannot signal.
    #[error("the frame's release is a timeline semaphore, not a sync fence")]
    TimelineRelease,
    /// The frame's colour contract has no plane equivalent.
    #[error(transparent)]
    Inexpressible(#[from] Inexpressible),
}

/// What a plane sets for a frame's buffer wherever it sits.
struct Contract {
    opaque: bool,
    dataspace: Dataspace,
    hdr: HdrMetadata,
}

/// The buffer contract of `frame` on a plane: the Android half of
/// [`Compositor::shows`].
///
/// # Errors
/// [`Ineligible`] naming the first reason the frame stays in engine
/// composition.
///
/// # Panics
/// On a poisoned release-sync mutex.
fn contract(frame: &vulkan::Frame) -> Result<Contract, Ineligible> {
    let generation = &frame.generation;
    let Some(source) = &generation.plane else {
        return Err(Ineligible::NotABuffer);
    };
    if !source.overlay {
        return Err(Ineligible::NoOverlayUsage);
    }
    if matches!(source.acquire, PlaneAcquire::Semaphore) {
        return Err(Ineligible::SemaphoreAcquire);
    }
    if matches!(
        *generation.release_sync.lock().expect("release sync"),
        Some(ReleaseSync::Timeline { .. })
    ) {
        return Err(Ineligible::TimelineRelease);
    }
    let encoding = match generation.repr {
        Repr::Rgb { format } => Encoding::Rgb {
            float: format == wgpu::TextureFormat::Rgba16Float,
            alpha: generation.alpha,
        },
        Repr::Planes { .. } | Repr::ExternalFormat { .. } => Encoding::Yuv,
    };
    Ok(Contract {
        opaque: plan::opaque(encoding)?,
        dataspace: plan::dataspace(&generation.color, encoding)?,
        hdr: source.hdr,
    })
}

/// The reason `frame`'s buffer cannot be shown on a plane, if any —
/// `contract`'s diagnostic half for reporting.
pub fn ineligible(frame: &vulkan::Frame) -> Option<Ineligible> {
    contract(frame).err()
}

/// The error a promoted layer's buffer or placement failure reports.
fn cannot_show(layer: LayerId, why: &dyn std::fmt::Display) -> RenderError {
    RenderError::Render(format!(
        "layer {layer:?} was promoted but a surface control cannot show it: {why}"
    ))
}

/// Duplicates a fence file descriptor: a buffer is set with its own fd,
/// so every acquirer gets a distinct one.
fn dup_fence(fd: &OwnedFd) -> Result<OwnedFd, RenderError> {
    fd.try_clone()
        .map_err(|e| RenderError::Render(format!("duplicate a fence: {e}")))
}

/// The stack `composition` describes, bottom first, with each promoted
/// frame's plane properties.
///
/// # Errors
/// [`RenderError`] when the plan promoted a layer whose buffer or placement
/// a plane cannot show.
fn stack<'a>(composition: &Composition<'a>) -> Result<Vec<Entry<'a>>, RenderError> {
    let mut entries = Vec::with_capacity(composition.parts.len() + composition.planes.len());
    for slot in plan::stack_order(composition.parts.len(), composition.planes.len()) {
        let plane = match slot {
            Slot::Part(n) => {
                entries.push(Entry::Engine {
                    view: Some(composition.parts[n].view),
                    size: composition.size,
                    raster: None,
                    properties: Properties {
                        z: 0,
                        placement: full(composition.size),
                        alpha: 1.0,
                        opaque: false,
                        dataspace: Dataspace::SRGB,
                        hdr: HdrMetadata::default(),
                    },
                });
                continue;
            }
            Slot::Plane(n) => &composition.planes[n],
        };
        let layer = plane.placement.layer;
        let (frame, generation) = match &plane.content {
            PlaneContent::Frame { frame, generation } => (frame, generation),
            PlaneContent::Raster { view, generation } => {
                entries.push(Entry::Engine {
                    view: *view,
                    size: plane.placement.size,
                    raster: Some((layer, *generation)),
                    properties: Properties {
                        z: 0,
                        placement: plan::promoted(plane.placement, composition.size)
                            .map_err(|e| cannot_show(layer, &e))?,
                        alpha: plane.placement.opacity,
                        opaque: false,
                        dataspace: plan::dataspace(
                            &crate::interop::FrameColor::LINEAR_P3,
                            Encoding::Rgb {
                                float: true,
                                alpha: crate::interop::RgbAlpha::Premultiplied,
                            },
                        )
                        .map_err(|e| cannot_show(layer, &e))?,
                        hdr: HdrMetadata::default(),
                    },
                });
                continue;
            }
        };
        let FramePlanes::Native(native) = &frame.planes else {
            return Err(cannot_show(layer, &Ineligible::NotABuffer));
        };
        let contract = contract(native).map_err(|e| cannot_show(layer, &e))?;
        let placement = plan::promoted(plane.placement, composition.size)
            .map_err(|e| cannot_show(layer, &e))?;
        entries.push(Entry::Frame(Promotion {
            layer,
            frame: native,
            generation: *generation,
            properties: Properties {
                z: 0,
                placement,
                alpha: plane.placement.opacity,
                opaque: contract.opaque,
                dataspace: contract.dataspace,
                hdr: contract.hdr,
            },
        }));
    }
    Ok(entries)
}

/// The whole-surface placement of an engine part.
const fn full(size: (u32, u32)) -> Placement {
    #[expect(
        clippy::cast_possible_wrap,
        reason = "surface sizes are bounded by the texture limit"
    )]
    let rect = IRect {
        left: 0,
        top: 0,
        right: size.0 as i32,
        bottom: size.1 as i32,
    };
    Placement::Visible(Geometry {
        source: rect,
        destination: rect,
        transform: BufferTransform {
            mirror_x: false,
            mirror_y: false,
            rotate_90: false,
        },
    })
}

/// A buffer whose release a completion reports.
struct Released {
    buffer: u64,
    fence: Option<OwnedFd>,
}

/// What a transaction replaced or removed on one surface control.
enum Replaced {
    /// One of the engine's own buffers.
    Buffer(u64),
    /// A promoted frame, still leased by its plane.
    Frame(vulkan::Frame),
    /// A surface control removed before it ever showed a buffer.
    Nothing,
}

/// One release a transaction's completion delivers.
struct Pending {
    /// The handle the replacement happened on.
    surface: *mut super::ffi::ASurfaceControl,
    what: Replaced,
    /// A removed surface control, released after its last query.
    removed: Option<SurfaceControl>,
}

// SAFETY: `surface` is never dereferenced: it is compared with the handles
// a completion reports, and passed to the NDK's thread-safe stats query only
// when the completion's own stats hold a reference to that surface control.
// Every other field is `Send`.
unsafe impl Send for Pending {}

/// One engine part's surface control and buffers.
struct Part {
    surface: SurfaceControl,
    buffers: Vec<Buffer>,
    /// The buffer the last transaction set.
    current: Option<u64>,
    uniform: wgpu::Buffer,
    shown: Option<Properties>,
    size: (u32, u32),
    raster: Option<(LayerId, u64)>,
    headroom: f32,
    format: wgpu::TextureFormat,
}

/// One promoted frame's surface control.
struct Plane {
    surface: SurfaceControl,
    /// The frame the last transaction set; its plane lease is held.
    frame: vulkan::Frame,
    /// The install count of `frame`.
    generation: u64,
    shown: Option<Properties>,
}

/// A `refresh` update validated against its plane and recorded in the
/// transaction — staged until the transaction holds every update, so an
/// error mid-way leaves `promoted` untouched (#90).
struct Update {
    layer: LayerId,
    generation: u64,
    /// The frame to lease and install — `None` keeps the plane's current
    /// one (its generation already matches).
    frame: Option<vulkan::Frame>,
    properties: Properties,
}

/// An engine surface realized as child surface controls of the host's
/// parent.
pub struct Planes {
    /// The engine's container under the host's parent: every part and plane
    /// is its child, so their z-orders never interleave with the host's.
    root: SurfaceControl,
    root_shown: bool,
    size: (u32, u32),
    transparent: bool,
    shared: Arc<Shared>,
    /// The engine's submit guard: a submission here carries queue waits
    /// and a signal of its own.
    submit_lock: Arc<Mutex<()>>,
    parts: Vec<Part>,
    /// The planes showing promoted frames, by layer.
    promoted: FxHashMap<LayerId, Plane>,
    /// `refresh`'s validated updates, staged while its transaction is
    /// built and drained once it is — the buffer is reused across calls.
    updates: Vec<Update>,
    raster_updates: Vec<(usize, Properties)>,
    /// Buffers of removed parts and previous sizes, dropped once released.
    retiring: Vec<Buffer>,
    next_buffer: u64,
    /// Signalled by each present submission; exported as the parts'
    /// acquire fence.
    signal: vk::Semaphore,
    released: mpsc::Receiver<Released>,
    release: mpsc::Sender<Released>,
    ops: Vec<Op>,
}

impl Planes {
    /// Creates the engine's container under `target`'s parent.
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when the device cannot import
    /// plane buffers or export sync fences, or the system refuses the
    /// surface control.
    pub fn new(
        shared: Arc<Shared>,
        submit_lock: Arc<Mutex<()>>,
        target: &SurfaceControlTarget,
    ) -> Result<Self, SurfaceError> {
        let unsupported =
            |why: &str| SurfaceError::UnsupportedTarget(format!("surface control: {why}"));
        if shared.vk.ahb.is_none() {
            return Err(unsupported(
                "AHardwareBuffer import is not enabled on the device",
            ));
        }
        if !shared.caps.external_semaphore_sync_fd {
            return Err(unsupported("the device cannot export sync fences"));
        }
        let root = target
            .parent
            .child(c"cherenkov")
            .map_err(|e| unsupported(&e.to_string()))?;
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let signal = unsafe {
            shared.vk.device.create_semaphore(
                &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                None,
            )
        }
        .map_err(|e| unsupported(&format!("sync-fence semaphore: {e}")))?;
        let (release, released) = mpsc::channel();
        Ok(Self {
            root,
            root_shown: false,
            size: target.size(),
            transparent: target.transparent,
            shared,
            submit_lock,
            parts: Vec::new(),
            promoted: FxHashMap::default(),
            updates: Vec::new(),
            raster_updates: Vec::new(),
            retiring: Vec::new(),
            next_buffer: 0,
            signal,
            released,
            release,
            ops: Vec::new(),
        })
    }

    /// Resizes the parts' buffers; buffers of the old size retire once the
    /// system compositor releases them.
    fn resize_parts(&mut self, size: (u32, u32)) {
        self.size = size;
        // Immutable captures keep their own local extent across resizes.
        for part in self.parts.iter_mut().filter(|part| part.raster.is_none()) {
            for buffer in part.buffers.drain(..) {
                if matches!(buffer.state, State::Shown) {
                    self.retiring.push(buffer);
                }
            }
        }
    }

    /// Takes the releases the system compositor delivered since the last
    /// frame.
    fn collect_releases(&mut self) {
        while let Ok(Released { buffer, fence }) = self.released.try_recv() {
            if let Some(found) = self
                .parts
                .iter_mut()
                .flat_map(|part| &mut part.buffers)
                .find(|b| b.id == buffer)
            {
                found.state = State::Free(fence);
            } else if let Some(at) = self.retiring.iter().position(|b| b.id == buffer) {
                self.retiring.swap_remove(at);
            }
        }
        for part in self.parts.iter_mut().filter(|part| part.raster.is_some()) {
            part.buffers
                .retain(|buffer| matches!(buffer.state, State::Shown));
        }
    }

    /// The index of a free buffer of `part` to draw into, allocating up to
    /// the pool size; `None` when every buffer is still with the system
    /// compositor.
    fn free_buffer(&mut self, part: usize) -> Result<Option<usize>, NativeError> {
        let buffers = &self.parts[part].buffers;
        if let Some(at) = buffers
            .iter()
            .position(|b| matches!(b.state, State::Free(_)))
        {
            return Ok(Some(at));
        }
        if buffers.len() == BUFFERS_PER_PART {
            return Ok(None);
        }
        let id = self.next_buffer;
        self.next_buffer += 1;
        let this = &self.parts[part];
        let buffer = if this.format == buffer::FORMAT {
            buffer::allocate(&self.shared, this.size, id)?
        } else {
            buffer::allocate_format(&self.shared, this.size, id, this.format)?
        };
        let buffers = &mut self.parts[part].buffers;
        buffers.push(buffer);
        Ok(Some(buffers.len() - 1))
    }

    /// Presents `composition` in one transaction. Returns `false` when a
    /// part has no free buffer yet: nothing changed on screen and the frame
    /// is retried.
    ///
    /// # Errors
    /// [`RenderError`] when a plane buffer cannot be allocated, a fence
    /// cannot be imported or exported, or a promoted layer is not showable.
    ///
    /// # Panics
    /// When the engine queue is not Vulkan.
    #[expect(
        clippy::too_many_lines,
        reason = "one frame's buffers, submission and transaction belong together"
    )]
    fn present(&mut self, composition: Composition<'_>) -> Result<bool, RenderError> {
        let native = |e: NativeError| RenderError::Render(format!("surface control: {e}"));
        let stack = stack(&composition)?;
        let Composition {
            device,
            queue,
            presenter,
            display,
            ..
        } = composition;
        self.collect_releases();
        let engine_parts = stack
            .iter()
            .filter(|entry| matches!(entry, Entry::Engine { .. }))
            .count();
        while self.parts.len() < engine_parts {
            let surface = self
                .root
                .child(c"cherenkov part")
                .map_err(|e| RenderError::Render(e.to_string()))?;
            self.parts.push(Part {
                surface,
                buffers: Vec::new(),
                current: None,
                uniform: Presenter::uniform(device),
                shown: None,
                size: self.size,
                raster: None,
                headroom: display.headroom,
                format: buffer::FORMAT,
            });
        }
        // Immutable captures follow layer identity, not their current slot
        // among engine parts. Their engine source is released on publication.
        for (index, raster) in stack
            .iter()
            .filter_map(|entry| match entry {
                Entry::Engine { raster, .. } => Some(raster),
                Entry::Frame(_) => None,
            })
            .enumerate()
        {
            if let Some((layer, _)) = raster
                && let Some(old) = self
                    .parts
                    .iter()
                    .position(|part| part.raster.is_some_and(|(id, _)| id == *layer))
            {
                self.parts.swap(index, old);
            }
        }
        let mut chosen = Vec::with_capacity(engine_parts);
        for (part, entry) in stack
            .iter()
            .filter(|entry| matches!(entry, Entry::Engine { .. }))
            .enumerate()
        {
            let Entry::Engine { size, raster, .. } = entry else {
                unreachable!("filtered engine entries");
            };
            let format = if raster.is_some() {
                wgpu::TextureFormat::Rgba16Float
            } else {
                buffer::FORMAT
            };
            let this = &mut self.parts[part];
            if this.size != *size || this.format != format {
                for buffer in this.buffers.drain(..) {
                    if matches!(buffer.state, State::Shown) {
                        self.retiring.push(buffer);
                    }
                }
                this.size = *size;
                this.format = format;
                this.raster = None;
            }
            if raster.is_some()
                && this.raster == *raster
                && this.current.is_some()
                && this.headroom.to_bits() == display.headroom.to_bits()
            {
                chosen.push(None);
            } else {
                match self.free_buffer(part).map_err(native)? {
                    Some(at) => chosen.push(Some(at)),
                    None => return Ok(false),
                }
            }
        }
        // Parts the plan no longer splits off leave in this frame's
        // transaction.
        let mut transaction = Transaction::new();
        let mut pending: Vec<Pending> = Vec::new();
        while self.parts.len() > engine_parts {
            let part = self.parts.pop().expect("more parts than the stack uses");
            for buffer in part.buffers {
                if matches!(buffer.state, State::Shown) {
                    self.retiring.push(buffer);
                }
            }
            transaction.reparent(&part.surface, None);
            pending.push(Pending {
                surface: part.surface.as_ptr(),
                what: part.current.map_or(Replaced::Nothing, Replaced::Buffer),
                removed: Some(part.surface),
            });
        }

        // One submission: acquire every chosen buffer, blit each part,
        // hand the buffers back, signal the exported fence.
        let images: Vec<vk::Image> = chosen
            .iter()
            .enumerate()
            .filter_map(|(part, &at)| at.map(|at| self.parts[part].buffers[at].image))
            .collect();
        let mut waits = Vec::new();
        for (part, &at) in chosen.iter().enumerate() {
            let Some(at) = at else {
                continue;
            };
            if let State::Free(fence) = &mut self.parts[part].buffers[at].state
                && let Some(fence) = fence.take()
            {
                waits.push(vulkan::import_sync_fd(&self.shared, fence).map_err(native)?);
            }
        }
        let barrier = |acquire: bool| {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("plane ownership"),
            });
            unsafe {
                encoder.as_hal_mut::<wgpu::hal::vulkan::Api, _, _>(|hal| {
                    let cb = hal.expect("the engine encoder is Vulkan").raw_handle();
                    buffer::ownership(&self.shared, cb, &images, acquire);
                });
            }
            encoder.finish()
        };
        let acquire = barrier(true);
        let mut blits = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("plane present"),
        });
        let mut sources = stack.iter().filter_map(|entry| match entry {
            Entry::Engine { view, raster, .. } => Some((*view, raster)),
            Entry::Frame(_) => None,
        });
        for (part, &at) in chosen.iter().enumerate() {
            let (source, raster) = sources.next().expect("one source per part");
            let Some(at) = at else {
                continue;
            };
            let output = TextureOutput {
                texture: &self.parts[part].buffers[at].texture,
                color: if raster.is_some() {
                    OutputColor::LinearDisplayP3
                } else {
                    OutputColor::Srgb
                },
                alpha: if part == 0 && !self.transparent {
                    OutputAlpha::Opaque
                } else {
                    OutputAlpha::Premultiplied
                },
                headroom: display.headroom,
            };
            queue.write_buffer(
                &self.parts[part].uniform,
                0,
                &Presenter::uniform_bytes(&output),
            );
            presenter.encode(
                device,
                &mut blits,
                source.expect("a new native buffer has engine pixels"),
                output,
                &self.parts[part].uniform,
                None,
            );
        }
        let blits = blits.finish();
        let hand_back = barrier(false);
        {
            let _submit = self.submit_lock.lock().expect("submit guard");
            let hal_queue =
                unsafe { queue.as_hal::<wgpu::hal::vulkan::Api>() }.expect("vulkan queue");
            for &wait in &waits {
                hal_queue.add_wait_semaphore(
                    wait,
                    None,
                    vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
                );
            }
            hal_queue.add_signal_semaphore(self.signal, None);
            queue.submit([acquire, blits, hand_back]);
        }
        crate::diag::submit(device, queue, "plane present");
        let acquire_fence = unsafe {
            self.shared
                .vk
                .external_semaphore_fd
                .as_ref()
                .expect("sync-fence export checked at creation")
                .get_semaphore_fd(
                    &vk::SemaphoreGetFdInfoKHR::default()
                        .semaphore(self.signal)
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                )
        }
        .map_err(|e| native(e.into()))?;
        // SAFETY: `get_semaphore_fd` returned a new descriptor we own.
        let acquire_fence =
            unsafe { <OwnedFd as std::os::fd::FromRawFd>::from_raw_fd(acquire_fence) };
        if !waits.is_empty() {
            let owner = self.shared.vk.device.clone();
            queue.on_submitted_work_done(move || {
                for wait in waits {
                    unsafe { owner.destroy_semaphore(wait, None) };
                }
            });
        }

        // The transaction: the stack's buffers and changed properties.
        if !self.root_shown {
            transaction.set(&self.root, Op::Visible(true));
            self.root_shown = true;
        }
        let mut part = 0;
        let mut seen = Vec::new();
        for (index, entry) in stack.iter().enumerate() {
            let z = plan::z_order(index);
            match entry {
                Entry::Engine {
                    raster, properties, ..
                } => {
                    let this = &mut self.parts[part];
                    if let Some(at) = chosen[part] {
                        let buffer = &mut this.buffers[at];
                        buffer.state = State::Shown;
                        unsafe {
                            transaction.set_buffer(
                                &this.surface,
                                buffer.ahb.0,
                                Some(dup_fence(&acquire_fence)?),
                            );
                        }
                        if let Some(previous) = this.current.replace(buffer.id) {
                            pending.push(Pending {
                                surface: this.surface.as_ptr(),
                                what: Replaced::Buffer(previous),
                                removed: None,
                            });
                        }
                    }
                    let properties = Properties {
                        z,
                        opaque: raster.is_none() && part == 0 && !self.transparent,
                        ..*properties
                    };
                    self.ops.clear();
                    plan::diff(this.shown.as_ref(), &properties, &mut self.ops);
                    for &op in &self.ops {
                        transaction.set(&this.surface, op);
                    }
                    this.shown = Some(properties);
                    this.raster = *raster;
                    this.headroom = display.headroom;
                    part += 1;
                }
                Entry::Frame(promotion) => {
                    let properties = Properties {
                        z,
                        ..promotion.properties
                    };
                    seen.push(promotion.layer);
                    let plane = match self.promoted.entry(promotion.layer) {
                        std::collections::hash_map::Entry::Occupied(slot) => slot.into_mut(),
                        std::collections::hash_map::Entry::Vacant(slot) => {
                            let surface = self
                                .root
                                .child(c"cherenkov frame")
                                .map_err(|e| RenderError::Render(e.to_string()))?;
                            promotion.frame.lease();
                            let plane = slot.insert(Plane {
                                surface,
                                frame: promotion.frame.clone(),
                                generation: promotion.generation,
                                shown: None,
                            });
                            set_frame(
                                &mut transaction,
                                &plane.surface,
                                promotion.frame,
                                dup_fence,
                            )?;
                            plane
                        }
                    };
                    if plane.generation != promotion.generation {
                        promotion.frame.lease();
                        plane.generation = promotion.generation;
                        let previous = std::mem::replace(&mut plane.frame, promotion.frame.clone());
                        set_frame(&mut transaction, &plane.surface, promotion.frame, dup_fence)?;
                        pending.push(Pending {
                            surface: plane.surface.as_ptr(),
                            what: Replaced::Frame(previous),
                            removed: None,
                        });
                    }
                    self.ops.clear();
                    plan::diff(plane.shown.as_ref(), &properties, &mut self.ops);
                    for &op in &self.ops {
                        transaction.set(&plane.surface, op);
                    }
                    plane.shown = Some(properties);
                }
            }
        }
        let gone: Vec<LayerId> = self
            .promoted
            .keys()
            .filter(|layer| !seen.contains(layer))
            .copied()
            .collect();
        for layer in gone {
            let plane = self.promoted.remove(&layer).expect("listed above");
            transaction.reparent(&plane.surface, None);
            pending.push(Pending {
                surface: plane.surface.as_ptr(),
                what: Replaced::Frame(plane.frame),
                removed: Some(plane.surface),
            });
        }

        Self::apply(transaction, &self.release, pending);
        Ok(true)
    }

    /// Applies `transaction`, delivering each `Pending` its release fence
    /// through the completion: an engine buffer's returns to its part's
    /// pool, a replaced frame's joins its producer's release before the
    /// plane's lease on it ends, and a removed surface control is dropped.
    fn apply(transaction: Transaction, release: &mpsc::Sender<Released>, pending: Vec<Pending>) {
        let sender = release.clone();
        transaction.apply(move |completion| {
            for Pending {
                surface,
                what,
                removed,
            } in pending
            {
                let fence = completion
                    .surfaces()
                    .contains(&surface)
                    .then(|| completion.previous_release(surface))
                    .flatten();
                match what {
                    Replaced::Buffer(id) => {
                        // A closed receiver means the surface is gone and
                        // its buffers with it.
                        sender.send(Released { buffer: id, fence }).ok();
                    }
                    Replaced::Frame(frame) => {
                        if let Some(fence) = fence {
                            frame.generation.add_plane_release(fence);
                        }
                        frame.unlease();
                    }
                    Replaced::Nothing => {}
                }
                drop(removed);
            }
        });
    }

    /// Presents only `frames`: one transaction that sets each promoted
    /// layer's new buffer with a duplicate of its acquire fence and any
    /// property the new frame's contract changes, leaving every part's
    /// shown buffer in place — no buffer is acquired, no submission runs
    /// and no fence is exported (#90).
    ///
    /// Every update is validated and recorded in the transaction before
    /// any bookkeeping moves, so an error leaves `promoted` describing
    /// the last applied transaction.
    ///
    /// # Errors
    /// [`RenderError`] when a promoted layer's buffer a plane cannot show,
    /// or a refresh names a plane that is not showing.
    fn refresh<'a>(
        &mut self,
        frames: impl Iterator<Item = crate::render::planes::Plane<'a>>,
    ) -> Result<(), RenderError> {
        self.collect_releases();
        // Validate every update and build the transaction before any
        // bookkeeping moves (as `present` does through `stack`): an error
        // leaves `promoted` describing the last applied transaction and
        // drops no staged release.
        self.updates.clear();
        self.raster_updates.clear();
        let mut transaction = Transaction::new();
        for update in frames {
            let layer = update.placement.layer;
            let (frame, generation) = match &update.content {
                PlaneContent::Frame { frame, generation } => (frame, generation),
                PlaneContent::Raster { generation, .. } => {
                    let (index, part) = self
                        .parts
                        .iter()
                        .enumerate()
                        .find(|(_, part)| part.raster == Some((layer, *generation)))
                        .expect("refresh names a committed raster generation");
                    let mut properties = part.shown.expect("committed raster properties");
                    properties.placement = plan::promoted(update.placement, self.size)
                        .map_err(|e| cannot_show(layer, &e))?;
                    properties.alpha = update.placement.opacity;
                    self.ops.clear();
                    plan::diff(part.shown.as_ref(), &properties, &mut self.ops);
                    for &op in &self.ops {
                        transaction.set(&part.surface, op);
                    }
                    self.raster_updates.push((index, properties));
                    continue;
                }
            };
            let FramePlanes::Native(native) = &frame.planes else {
                return Err(cannot_show(layer, &Ineligible::NotABuffer));
            };
            let contract = contract(native).map_err(|e| cannot_show(layer, &e))?;
            let Some(plane) = self.promoted.get(&layer) else {
                return Err(RenderError::Render(format!(
                    "layer {layer:?}'s frame changed while no plane shows it"
                )));
            };
            // Admission preserves stack membership and z-order; poses and
            // opacity can change without replacing the buffer. `shown`
            // is `None` only when a previous present failed mid-
            // transaction — report it rather than touch the surface
            // control with half its properties.
            let Some(mut properties) = plane.shown else {
                return Err(RenderError::Render(format!(
                    "layer {layer:?}'s plane never showed a frame"
                )));
            };
            let frame = if plane.generation == *generation {
                None
            } else {
                set_frame(&mut transaction, &plane.surface, native, dup_fence)?;
                Some(native.clone())
            };
            properties.opaque = contract.opaque;
            properties.placement =
                plan::promoted(update.placement, self.size).map_err(|e| cannot_show(layer, &e))?;
            properties.alpha = update.placement.opacity;
            properties.dataspace = contract.dataspace;
            properties.hdr = contract.hdr;
            self.ops.clear();
            plan::diff(plane.shown.as_ref(), &properties, &mut self.ops);
            for &op in &self.ops {
                transaction.set(&plane.surface, op);
            }
            self.updates.push(Update {
                layer,
                generation: *generation,
                frame,
                properties,
            });
        }
        let mut pending = Vec::with_capacity(self.updates.len());
        for update in self.updates.drain(..) {
            let plane = self
                .promoted
                .get_mut(&update.layer)
                .expect("validated above");
            if let Some(frame) = update.frame {
                frame.lease();
                plane.generation = update.generation;
                let previous = std::mem::replace(&mut plane.frame, frame);
                pending.push(Pending {
                    surface: plane.surface.as_ptr(),
                    what: Replaced::Frame(previous),
                    removed: None,
                });
            }
            plane.shown = Some(update.properties);
        }
        for (index, properties) in self.raster_updates.drain(..) {
            self.parts[index].shown = Some(properties);
        }
        Self::apply(transaction, &self.release, pending);
        Ok(())
    }

    /// Removes every part and plane from the display; their releases are
    /// delivered by the transaction's completion.
    fn clear(&mut self) {
        let mut transaction = Transaction::new();
        let mut frames = Vec::new();
        for (_, plane) in self.promoted.drain() {
            transaction.reparent(&plane.surface, None);
            frames.push((plane.surface, plane.frame));
        }
        let mut surfaces = Vec::new();
        for part in self.parts.drain(..) {
            transaction.reparent(&part.surface, None);
            surfaces.push(part.surface);
        }
        transaction.reparent(&self.root, None);
        transaction.apply(move |completion| {
            for (surface, frame) in frames {
                let raw = surface.as_ptr();
                if completion.surfaces().contains(&raw)
                    && let Some(fence) = completion.previous_release(raw)
                {
                    frame.generation.add_plane_release(fence);
                }
                frame.unlease();
            }
            drop(surfaces);
        });
    }
}

impl Compositor for Planes {
    const BUDGET: usize = plan::BUDGET;

    fn expresses_transform(transform: Affine) -> bool {
        plan::expresses_transform(transform)
    }

    fn expresses_clip(clip: &ShapeData) -> bool {
        plan::expresses_clip(clip)
    }

    fn shows(frame: &ExternalFrame) -> bool {
        matches!(&frame.planes, FramePlanes::Native(native) if contract(native).is_ok())
    }
}

impl SystemPlanes for Planes {
    fn captured_bytes(&self) -> u64 {
        self.parts
            .iter()
            .filter(|part| part.raster.is_some())
            .flat_map(|part| &part.buffers)
            .map(|buffer| buffer.bytes)
            .sum()
    }

    fn compose(
        &mut self,
        composition: Composition<'_>,
    ) -> Result<crate::render::planes::Presentation, RenderError> {
        self.present(composition).map(|shown| {
            if shown {
                crate::render::planes::Presentation::Presented
            } else {
                crate::render::planes::Presentation::Retry
            }
        })
    }

    fn refresh<'a>(
        &mut self,
        frames: impl Iterator<Item = crate::render::planes::Plane<'a>>,
    ) -> Result<(), RenderError> {
        Self::refresh(self, frames)
    }

    fn resize(&mut self, size: (u32, u32)) {
        self.resize_parts(size);
    }

    fn reselect(&mut self, _: &wgpu::Adapter, _: &wgpu::Device) -> Result<(), SurfaceError> {
        // Engine parts are RGBA8 `AHardwareBuffer`s in the sRGB dataspace.
        // That contract is not a swapchain negotiation, and a frame's
        // headroom is read when the part is presented.
        Ok(())
    }
}

impl Drop for Planes {
    fn drop(&mut self) {
        self.clear();
        unsafe { self.shared.vk.device.destroy_semaphore(self.signal, None) };
    }
}

/// Sets `frame`'s buffer on `surface` with a duplicate of its acquire fence.
fn set_frame(
    transaction: &mut Transaction,
    surface: &SurfaceControl,
    frame: &vulkan::Frame,
    dup: impl Fn(&OwnedFd) -> Result<OwnedFd, RenderError>,
) -> Result<(), RenderError> {
    let source = frame
        .generation
        .plane
        .as_ref()
        .expect("eligibility checked the plane source");
    let fence = match &source.acquire {
        PlaneAcquire::Ready => None,
        PlaneAcquire::Fence(fd) => Some(dup(fd)?),
        PlaneAcquire::Semaphore => unreachable!("eligibility rejects semaphore acquires"),
    };
    // SAFETY: the generation's producer lease keeps the buffer alive, and
    // the plane's own lease keeps the generation's lease.
    unsafe { transaction.set_buffer(surface, source.buffer, fence) };
    Ok(())
}
