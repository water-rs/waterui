//! The Linux DMA-BUF export target (#1687).
//!
//! A [`Pool`] is a bounded set of Vulkan images the engine renders its
//! presentation output into and exports as dma-bufs. The negotiation is
//! honest throughout: the `(fourcc, modifier)` pair comes from the
//! host's declared [`DmabufFormat`] list, checked against the device —
//! `VkImageDrmFormatModifierListCreateInfoEXT` where the modifier
//! extension exists, `VK_IMAGE_TILING_LINEAR` for hosts that declare
//! `DRM_FORMAT_MOD_LINEAR` — and the delivered planes carry the
//! driver's own `vkGetImageSubresourceLayout` offsets and strides.
//!
//! Synchronisation is all GPU-side: every presented frame's `acquire`
//! is a `SYNC_FD` semaphore exported from the submission that wrote the
//! image, and a host's release fence is imported into a one-shot
//! semaphore that the image's next writing submission waits on. Nothing
//! blocks the render thread — a surface with every image out waits for
//! a release by asking for another frame (`Presentation::Retry`).

#![cfg(target_os = "linux")]

use std::os::fd::FromRawFd;
use std::os::unix::io::OwnedFd;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use ash::vk;

use cherenkov::{RenderError, SurfaceError};

use crate::interop::dmabuf::{DRM_FORMAT_MOD_LINEAR, DmabufFrame, DmabufPlane, DmabufTarget};
use crate::interop::{FrameColor, OutputAlpha, OutputColor, Primaries, RgbAlpha, Transfer};
use crate::render::external::vulkan::{self, NativeError, Shared};
use crate::render::{planes, present};

/// The host's return of one presented image: the slot it was delivered
/// from and the sync file the engine waits on before writing it again.
pub struct Release {
    /// The pool slot the returned image was presented from.
    pub slot: usize,
    /// The host's release sync file.
    pub fence: OwnedFd,
}

/// One pool image: the raw Vulkan objects, their wgpu wrapper (the
/// render attachment [`present::Presenter`] draws into) and the exported
/// plane layout the delivered frames carry.
struct Slot {
    image: vk::Image,
    memory: vk::DeviceMemory,
    texture: wgpu::Texture,
    /// `(offset, stride)` of every colour plane, queried from the image.
    planes: Vec<(u32, u32)>,
    /// What the host owes back: released slots carry their release fence
    /// as an imported one-shot semaphore — the slot is reusable, but the
    /// reuse waits on this semaphore on the GPU.
    wait: Option<vk::Semaphore>,
    /// Whether the image is out with the host.
    out: bool,
}

/// The negotiated output: the (fourcc, modifier, `VkFormat`) triple the
/// pool allocates with, and the encoding the present pass writes.
struct Negotiated {
    fourcc: u32,
    modifier: u64,
    format: vk::Format,
    wgpu: wgpu::TextureFormat,
    color: OutputColor,
    alpha: OutputAlpha,
}

/// The colour description a presented frame carries — the decode half
/// of the [`OutputColor`] the pass encoded with.
const fn presented_color(color: OutputColor, alpha: OutputAlpha) -> (FrameColor, RgbAlpha) {
    let (primaries, transfer, hlg_peak) = match color {
        OutputColor::Srgb | OutputColor::ExtendedSrgb => (Primaries::Bt709, Transfer::Srgb, 1.0),
        OutputColor::DisplayP3 | OutputColor::ExtendedDisplayP3 => {
            (Primaries::DisplayP3, Transfer::Srgb, 1.0)
        }
        OutputColor::LinearDisplayP3 => (Primaries::DisplayP3, Transfer::Linear, 1.0),
        OutputColor::ExtendedSrgbLinear => (Primaries::Bt709, Transfer::Linear, 1.0),
        OutputColor::Bt2100Pq => (Primaries::Bt2020, Transfer::Pq, 1.0),
        // The engine's HLG output targets the BT.2100 1000-nit nominal
        // peak under the reference OOTF (#98).
        OutputColor::Bt2100Hlg => (Primaries::Bt2020, Transfer::Hlg, 1000.0),
    };
    let alpha = match alpha {
        OutputAlpha::Opaque => RgbAlpha::Opaque,
        OutputAlpha::Premultiplied => RgbAlpha::Premultiplied,
        OutputAlpha::Straight => RgbAlpha::Straight,
    };
    (
        FrameColor {
            matrix: crate::interop::YuvMatrix::Bt709,
            range: crate::interop::YuvRange::Full,
            chroma_siting: crate::interop::ChromaSiting::CENTERED,
            primaries,
            transfer,
            reference_white: present::REFERENCE_WHITE_NITS,
            hlg_peak,
        },
        alpha,
    )
}

/// The fourccs this target exports: single-plane RGB(A) only — a
/// multiplanar format has no renderable `vk::Format` and belongs to the
/// import side's contract, not this one's.
const fn export_format(fourcc: u32) -> Option<(vk::Format, wgpu::TextureFormat)> {
    use crate::interop::dmabuf as drm;
    Some(match fourcc {
        f if f == drm::DRM_FORMAT_ARGB8888 || f == drm::DRM_FORMAT_XRGB8888 => {
            (vk::Format::B8G8R8A8_UNORM, wgpu::TextureFormat::Bgra8Unorm)
        }
        f if f == drm::DRM_FORMAT_ABGR8888 || f == drm::DRM_FORMAT_XBGR8888 => {
            (vk::Format::R8G8B8A8_UNORM, wgpu::TextureFormat::Rgba8Unorm)
        }
        f if f == drm::DRM_FORMAT_ABGR16161616F => (
            vk::Format::R16G16B16A16_SFLOAT,
            wgpu::TextureFormat::Rgba16Float,
        ),
        _ => return None,
    })
}

/// Queries the physical device for the exact format/modifier/usage/
/// handle combination the export needs before any object is created —
/// the export-side mirror of the importer's `support_checked`.
fn export_supported(shared: &Shared, format: vk::Format, modifier: u64) -> Result<(), NativeError> {
    let usage = vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED;
    let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let linear_only = modifier == DRM_FORMAT_MOD_LINEAR && !shared.caps.image_drm_format_modifier;
    // Without the modifier extension a `tiling = LINEAR` image's exported
    // layout is exactly what `DRM_FORMAT_MOD_LINEAR` names; any other
    // modifier is unsupported and the query below is skipped — the
    // extension is what makes a modifier expressible.
    if !linear_only && !shared.caps.image_drm_format_modifier {
        return Err(NativeError::Unsupported(
            "VK_EXT_image_drm_format_modifier is not enabled",
        ));
    }
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(if linear_only {
            vk::ImageTiling::LINEAR
        } else {
            vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT
        })
        .usage(usage)
        .push_next(&mut external_info);
    let info = if linear_only {
        info
    } else {
        info.push_next(&mut modifier_info)
    };
    let mut props = vk::ImageFormatProperties2::default();
    // SAFETY: `shared`'s instance/physical_device are live and `info`/
    // `props` are correctly-chained parameter and out structs.
    unsafe {
        shared
            .instance
            .get_physical_device_image_format_properties2(shared.physical_device, &info, &mut props)
    }
    .map_err(|_| NativeError::Unsupported("format/modifier rejected by driver"))?;
    Ok(())
}

/// The memory type for an exportable allocation: any type the image
/// requires — a dma-buf export carries no flag constraints of its own.
fn find_memory_type(shared: &Shared, bits: u32) -> Result<u32, NativeError> {
    let mut props = vk::PhysicalDeviceMemoryProperties2::default();
    // SAFETY: `shared`'s instance/physical_device are live and `props`
    // is a live out struct.
    unsafe {
        shared
            .instance
            .get_physical_device_memory_properties2(shared.physical_device, &mut props);
    }
    (0..props.memory_properties.memory_type_count)
        .find(|&i| bits & (1 << i) != 0)
        .ok_or(NativeError::Unsupported(
            "the image requires no exportable memory type",
        ))
}

/// Creates one pool image: exportable memory behind a
/// `DRM_FORMAT_MODIFIER_EXT`- or `LINEAR`-tiled `VkImage`, wrapped as a
/// `wgpu::Texture` the present pass renders into.
#[expect(
    clippy::too_many_lines,
    reason = "one image's create/bind/query/wrap sequence — every error path must first destroy what it created"
)]
fn create_slot(
    shared: &Shared,
    label: &'static str,
    size: (u32, u32),
    negotiated: &Negotiated,
) -> Result<Slot, NativeError> {
    let dev = &shared.vk.device;
    // A non-LINEAR modifier is expressible only through the extension;
    // with it present even LINEAR is created through the modifier path,
    // the honest explicit modifier.
    let explicit = shared.caps.image_drm_format_modifier;
    debug_assert!(explicit || negotiated.modifier == DRM_FORMAT_MOD_LINEAR);
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut modifiers = vk::ImageDrmFormatModifierListCreateInfoEXT::default()
        .drm_format_modifiers(std::slice::from_ref(&negotiated.modifier));
    let mut info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(negotiated.format)
        .extent(vk::Extent3D {
            width: size.0,
            height: size.1,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(if explicit {
            vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT
        } else {
            vk::ImageTiling::LINEAR
        })
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external);
    if explicit {
        info = info.push_next(&mut modifiers);
    }
    // SAFETY: `dev` is live; `info` chains the external-memory handle
    // type and, when negotiated, the single modifier the image is
    // created with — a one-element list forces that modifier.
    let image = unsafe { dev.create_image(&info, None) }.map_err(NativeError::from)?;
    // SAFETY: `image` is a live image created above.
    let requirements = unsafe { dev.get_image_memory_requirements(image) };
    let memory_type = match find_memory_type(shared, requirements.memory_type_bits) {
        Ok(index) => index,
        Err(err) => {
            // SAFETY: `image` was created above and is otherwise
            // unreferenced.
            unsafe { dev.destroy_image(image, None) };
            return Err(err);
        }
    };
    let mut export_alloc = vk::ExportMemoryAllocateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(requirements.size)
        .memory_type_index(memory_type)
        .push_next(&mut export_alloc);
    if shared.caps.dedicated_allocation {
        alloc = alloc.push_next(&mut dedicated);
    }
    // SAFETY: `dev` is live and `alloc` chains the export handle type
    // plus the dedicated binding when the driver supports it.
    let memory = unsafe { dev.allocate_memory(&alloc, None) }.map_err(|err| {
        // SAFETY: `image` was created above and is otherwise
        // unreferenced.
        unsafe { dev.destroy_image(image, None) };
        NativeError::from(err)
    })?;
    // SAFETY: `memory` was allocated for `image`, sized to its
    // requirements.
    if let Err(err) = unsafe { dev.bind_image_memory(image, memory, 0) } {
        // SAFETY: `image`/`memory` were created as a pair above and are
        // otherwise unreferenced — destroyed exactly once here.
        unsafe {
            dev.free_memory(memory, None);
            dev.destroy_image(image, None);
        }
        return Err(err.into());
    }
    // The delivered planes carry the driver's own layout: a modifier
    // image answers `MEMORY_PLANE_0` for its single memory plane — the
    // spec's required aspect for `DRM_FORMAT_MODIFIER_EXT` tiling — and
    // a linear image answers `COLOR`.
    let aspect = if explicit {
        vk::ImageAspectFlags::MEMORY_PLANE_0_EXT
    } else {
        vk::ImageAspectFlags::COLOR
    };
    // SAFETY: `image` is live and single-plane, so its aspect carries
    // exactly the subresource queried.
    let layout = unsafe {
        dev.get_image_subresource_layout(
            image,
            vk::ImageSubresource::default()
                .aspect_mask(aspect)
                .mip_level(0)
                .array_layer(0),
        )
    };
    let planes = vec![(
        u32::try_from(layout.offset).expect("plane offset fits u32"),
        u32::try_from(layout.row_pitch).expect("plane stride fits u32"),
    )];
    // The wgpu wrapper the present pass attaches: `External` memory keeps
    // ownership with the slot, the drop guard owns nothing, and the
    // image starts `UNINITIALIZED` so wgpu's first attachment pass
    // transitions it honestly.
    // SAFETY: `shared.device`/`image` are live, `image` was created for
    // this device with `usage = COLOR_ATTACHMENT | SAMPLED` in `UNORM`/
    // `SFLOAT`, matching the `wgpu::TextureFormat` the descriptor
    // names; the slot owns the `VkImage` so the drop guard is empty.
    let texture = unsafe {
        let hal_device = shared
            .wgpu
            .as_hal::<wgpu::hal::vulkan::Api>()
            .expect("vulkan device");
        let hal_tex = hal_device.texture_from_raw(
            image,
            &wgpu::hal::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width: size.0,
                    height: size.1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: negotiated.wgpu,
                usage: wgpu::wgt::TextureUses::COLOR_TARGET,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            },
            Some(Box::new(|| {})),
            wgpu::hal::vulkan::TextureMemory::External,
        );
        shared
            .wgpu
            .create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                hal_tex,
                &wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: size.0,
                        height: size.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: negotiated.wgpu,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::wgt::TextureUses::UNINITIALIZED,
            )
    };
    Ok(Slot {
        image,
        memory,
        texture,
        planes,
        wait: None,
        out: false,
    })
}

/// Picks the first host-declared `(fourcc, modifier)` the device
/// supports. No candidates passing is `UnsupportedTarget` naming every
/// rejection — fail fast, never a fallback the host did not declare.
fn negotiate(shared: &Shared, target: &DmabufTarget) -> Result<Negotiated, SurfaceError> {
    let mut tried = Vec::new();
    for format in &target.formats {
        let Some((vk_format, wgpu_format)) = export_format(format.fourcc) else {
            tried.push(format!(
                "fourcc {:#x} has no renderable export format",
                format.fourcc
            ));
            continue;
        };
        for &modifier in &format.modifiers {
            match export_supported(shared, vk_format, modifier) {
                Ok(()) => {
                    return Ok(Negotiated {
                        fourcc: format.fourcc,
                        modifier,
                        format: vk_format,
                        wgpu: wgpu_format,
                        color: format.color,
                        alpha: format.alpha,
                    });
                }
                Err(err) => tried.push(format!(
                    "fourcc {:#x} modifier {:#x}: {err}",
                    format.fourcc, modifier
                )),
            }
        }
    }
    Err(SurfaceError::UnsupportedTarget(format!(
        "no declared dma-buf format is exportable{}",
        if tried.is_empty() {
            " (none declared)".into()
        } else {
            format!(": {}", tried.join("; "))
        }
    )))
}

/// One surface's bounded pool of exportable images.
///
/// Created at surface creation; `present` runs on the render thread,
/// `release` arrives on the host's thread over `returns`.
pub struct Pool {
    shared: Arc<Shared>,
    /// Serialises `add_wait_semaphore`/`add_signal_semaphore` staging
    /// against the submissions that consume them — the same guard the
    /// external-frame submit path holds.
    submit_lock: Arc<Mutex<()>>,
    sink: Sender<DmabufFrame>,
    release_to: Sender<Release>,
    returns: Receiver<Release>,
    negotiated: Negotiated,
    /// The size the slots are allocated at.
    size: (u32, u32),
    /// The negotiated image count — `DmabufTarget`'s bound.
    count: usize,
    slots: Vec<Slot>,
    /// One-shot release semaphores staged onto the latest submission —
    /// destroyed when the queue reports that submission complete.
    spent_waits: Vec<vk::Semaphore>,
    /// Set when a resize recreation failed; every subsequent present
    /// reports it — fail fast, no degraded path.
    broken: Option<String>,
    /// The sink died; warn once, keep returning `Retry` forever.
    sink_dead: bool,
}

impl std::fmt::Debug for Pool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pool")
            .field("size", &self.size)
            .field("pool_size", &self.count)
            .field("fourcc", &self.negotiated.fourcc)
            .field("modifier", &self.negotiated.modifier)
            .finish_non_exhaustive()
    }
}

impl Pool {
    /// Negotiates `target`'s formats against the device and allocates
    /// the pool.
    ///
    /// # Errors
    /// [`SurfaceError::UnsupportedTarget`] when the device lacks dma-buf
    /// export or sync-fd semaphores, or when no declared format works.
    pub fn new(
        shared: &Arc<Shared>,
        submit_lock: Arc<Mutex<()>>,
        target: &DmabufTarget,
    ) -> Result<Self, SurfaceError> {
        if !(shared.caps.external_memory_fd && shared.caps.external_memory_dma_buf) {
            return Err(SurfaceError::UnsupportedTarget(
                "dma-buf export needs VK_KHR_external_memory_fd and \
                 VK_EXT_external_memory_dma_buf"
                    .into(),
            ));
        }
        if !shared.caps.external_semaphore_sync_fd {
            return Err(SurfaceError::UnsupportedTarget(
                "acquire/release sync files need SYNC_FD semaphore import+export".into(),
            ));
        }
        let negotiated = negotiate(shared, target)?;
        let (release_to, returns) = std::sync::mpsc::channel();
        let mut pool = Self {
            shared: Arc::clone(shared),
            submit_lock,
            sink: target.sink.clone(),
            release_to,
            returns,
            negotiated,
            size: target.size,
            count: target.pool_size,
            slots: Vec::new(),
            spent_waits: Vec::new(),
            broken: None,
            sink_dead: false,
        };
        pool.allocate()
            .map_err(|err| SurfaceError::UnsupportedTarget(format!("dma-buf pool: {err}")))?;
        Ok(pool)
    }

    /// Allocates the slot set at `self.size`, destroying any existing
    /// slots — a resize or teardown. Held-out images keep living in the
    /// host's fds; their releases die on the shared return channel.
    fn allocate(&mut self) -> Result<(), NativeError> {
        self.destroy_slots();
        let mut slots = Vec::with_capacity(self.count);
        for _ in 0..self.count {
            let slot = create_slot(&self.shared, "dma-buf export", self.size, &self.negotiated)
                .inspect_err(|_| {
                    for slot in std::mem::take(&mut slots) {
                        destroy_slot(&self.shared, slot);
                    }
                })?;
            slots.push(slot);
        }
        self.slots = slots;
        Ok(())
    }

    /// Drops every slot's objects and every pending release semaphore.
    fn destroy_slots(&mut self) {
        // Releases the host already sent die with the slots they were
        // sent for — every slot is being recreated or destroyed.
        while self.returns.try_recv().is_ok() {}
        for slot in self.slots.drain(..) {
            destroy_slot(&self.shared, slot);
        }
        for semaphore in self.spent_waits.drain(..) {
            // SAFETY: a spent release semaphore is never staged again —
            // its one destroy.
            unsafe {
                self.shared.vk.device.destroy_semaphore(semaphore, None);
            }
        }
    }

    /// The surface resized; the pool is rebuilt at the new size, and a
    /// failure is latched for the next present rather than swallowed.
    pub fn resize(&mut self, size: (u32, u32)) {
        self.size = size;
        self.broken = self.allocate().err().map(|err| err.to_string());
    }

    /// Presents `source` into a free pool image and delivers it to the
    /// sink — [`Presentation::Retry`] when every image is out with the
    /// host, which is the bounded pool's whole contract.
    ///
    /// # Errors
    /// [`RenderError`] when a release fence cannot be imported, the
    /// fence export or memory-fd export fails, or a resize failed.
    pub fn present(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        presenter: &mut present::Presenter,
        source: &wgpu::TextureView,
        headroom: f32,
    ) -> Result<planes::Presentation, RenderError> {
        if let Some(error) = &self.broken {
            return Err(RenderError::Render(format!("dma-buf pool: {error}")));
        }
        self.drain_releases()?;
        let Some(slot) = self.slots.iter().position(|slot| !slot.out) else {
            return Ok(planes::Presentation::Retry);
        };
        let dev = &self.shared.vk.device;
        // The acquire fence the host will wait on: a `SYNC_FD`-exportable
        // binary semaphore, fresh per presentation — its pending payload
        // is consumed by the export, so it cannot be reused.
        let mut export = vk::ExportSemaphoreCreateInfo::default()
            .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
        let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
        // SAFETY: `dev` is live and `info` chains the SYNC_FD
        // declaration.
        let acquire_sem = unsafe { dev.create_semaphore(&info, None) }
            .map_err(|err| RenderError::Render(format!("acquire semaphore: {err:?}")))?;
        {
            // SAFETY: `queue` is the engine's own queue; Vulkan is
            // asserted by `expect`.
            let hal_queue =
                unsafe { queue.as_hal::<wgpu::hal::vulkan::Api>() }.expect("vulkan queue");
            let _submit = self.submit_lock.lock().expect("submit guard");
            if let Some(wait) = self.slots[slot].wait.take() {
                // The host's release fence orders the image's next write
                // behind its consumers — on the GPU, never on the CPU.
                hal_queue.add_wait_semaphore(wait, None, vk::PipelineStageFlags::ALL_COMMANDS);
                self.spent_waits.push(wait);
            }
            // The present submission signals the frame's acquire fence.
            hal_queue.add_signal_semaphore(acquire_sem, None);
            presenter.texture(
                device,
                queue,
                source,
                present::TextureOutput {
                    texture: &self.slots[slot].texture,
                    color: self.negotiated.color,
                    alpha: self.negotiated.alpha,
                    headroom,
                },
            );
        }
        // The submission is accepted and the signal still pending — the
        // exact moment the external-frame release path exports its fence
        // (#166). A SYNC_FD export consumes the semaphore's payload, so
        // the object dies immediately after the fd is taken.
        let loader = self
            .shared
            .vk
            .external_semaphore_fd
            .as_ref()
            .expect("sync-fd export checked at creation");
        // SAFETY: `acquire_sem` is live, created with the `SYNC_FD`
        // handle type, and signalled by the pending submission — the
        // exact window the export is defined for.
        let acquired = unsafe {
            loader.get_semaphore_fd(
                &vk::SemaphoreGetFdInfoKHR::default()
                    .semaphore(acquire_sem)
                    .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
            )
        };
        // SAFETY: `acquire_sem` was created above and its payload moved
        // to the sync file (or the export failed and it is unreferenced)
        // — destroyed exactly once here.
        unsafe { dev.destroy_semaphore(acquire_sem, None) };
        let acquired =
            acquired.map_err(|err| RenderError::Render(format!("acquire fence export: {err}")))?;
        // SAFETY: `get_semaphore_fd` returned a new fd owned by the
        // caller.
        let acquire = unsafe { OwnedFd::from_raw_fd(acquired) };
        // One dma-buf fd per colour plane — every format this pool
        // negotiates is single-plane, so one export plus `try_clone`
        // serves any count.
        let frame_planes = self.slots[slot]
            .planes
            .iter()
            .map(|&(offset, stride)| {
                let fd = self.memory_fd(slot)?;
                Ok(DmabufPlane { fd, offset, stride })
            })
            .collect::<Result<Vec<_>, RenderError>>()?;
        let (color, alpha) = presented_color(self.negotiated.color, self.negotiated.alpha);
        let frame = DmabufFrame {
            planes: frame_planes,
            fourcc: self.negotiated.fourcc,
            modifier: self.negotiated.modifier,
            size: self.size,
            layout: u32::try_from(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL.as_raw())
                .expect("layout code is positive"),
            color,
            alpha,
            acquire,
            release_to: self.release_to.clone(),
            slot,
        };
        self.slots[slot].out = true;
        if self.sink.send(frame).is_err() && !self.sink_dead {
            tracing::warn!("dma-buf sink dropped; the surface will not present again");
            self.sink_dead = true;
        }
        if !self.spent_waits.is_empty() {
            // The release semaphores the submission consumed are
            // destroyed once the queue retires that submission — the
            // deferred-destroy contract the engine's own release path
            // uses.
            let dev = self.shared.vk.device.clone();
            let spent = std::mem::take(&mut self.spent_waits);
            queue.on_submitted_work_done(move || {
                for semaphore in spent {
                    // SAFETY: each semaphore's one wait was consumed by
                    // the submission this callback retires.
                    unsafe { dev.destroy_semaphore(semaphore, None) };
                }
            });
        }
        Ok(planes::Presentation::Presented)
    }

    /// Imports every release fence the host returned into a one-shot
    /// semaphore on the freed slot — the GPU-side wait the next write
    /// stages.
    fn drain_releases(&mut self) -> Result<(), RenderError> {
        while let Ok(release) = self.returns.try_recv() {
            let Some(slot) = self.slots.get_mut(release.slot) else {
                continue;
            };
            let wait = vulkan::import_sync_fd(&self.shared, release.fence)
                .map_err(|err| RenderError::Render(format!("release fence import: {err}")))?;
            // A slot released twice is a host bug; the later fence wins
            // and the earlier semaphore dies unstaged — still a destroy,
            // never a wait.
            if let Some(previous) = slot.wait.replace(wait) {
                // SAFETY: `previous` was imported for this slot and never
                // staged — destroyed exactly once.
                unsafe {
                    self.shared.vk.device.destroy_semaphore(previous, None);
                }
            }
            slot.out = false;
        }
        Ok(())
    }

    /// A fresh dma-buf fd for `slot`'s memory — the exported handle the
    /// delivered plane carries.
    fn memory_fd(&self, slot: usize) -> Result<OwnedFd, RenderError> {
        let loader = self
            .shared
            .vk
            .external_memory_fd
            .as_ref()
            .expect("dma-buf export checked at creation");
        // SAFETY: `self.slots[slot].memory` was allocated with the
        // `DMA_BUF_EXT` export handle type.
        let fd = unsafe {
            loader.get_memory_fd(
                &vk::MemoryGetFdInfoKHR::default()
                    .memory(self.slots[slot].memory)
                    .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT),
            )
        }
        .map_err(|err| RenderError::Render(format!("dma-buf fd export: {err}")))?;
        // SAFETY: `get_memory_fd` returned a new fd owned by the caller.
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.destroy_slots();
    }
}

/// Destroys one slot's objects: the wgpu wrapper drops its handle (its
/// drop guard owns nothing), then the image and memory die together.
fn destroy_slot(shared: &Shared, slot: Slot) {
    drop(slot.texture);
    let dev = &shared.vk.device;
    // SAFETY: `image`/`memory` were created as a pair on `dev` and are
    // destroyed exactly once — here or in `allocate`'s error path.
    unsafe {
        dev.destroy_image(slot.image, None);
        dev.free_memory(slot.memory, None);
    }
    if let Some(wait) = slot.wait {
        // SAFETY: `wait` was imported for this slot and never staged.
        unsafe { dev.destroy_semaphore(wait, None) };
    }
}
