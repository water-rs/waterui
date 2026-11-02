//! The Vulkan tile executor's mirror of the engine's wgpu objects.
//!
//! wgpu-hal keeps the handles the native executor needs private:
//! `PipelineLayout.raw` and `BindGroup.set` are not reachable from a
//! `wgpu::PipelineLayout` or `wgpu::BindGroup`, and the fork is not
//! changed to expose them. So the executor owns a *mirror*: descriptor
//! set layouts that match `ENGINE_GROUP0`/`ENGINE_GROUP1` binding for
//! binding, a third set for the two input attachments the rewritten
//! tile SPIR-V reads, one pipeline layout over the three, and the
//! pipelines the epoch's draws bind. The descriptor sets it allocates
//! are written from the raw handles behind the engine's own wgpu
//! buffers and views (`Buffer::raw_handle`, `TextureView::raw_handle`),
//! so both sides name the same memory.
//!
//! # Lifetime of the mirror
//!
//! The immutable half — shader modules, the three set layouts, the
//! pipeline layout, the pipeline cache — is created once per
//! [`Mirror`] and lives until the renderer drops it. A pipeline is
//! created on first use of its key and never invalidated: its key names
//! only the closed shader set, the slot count and the slot roles, none
//! of which depend on a resource identity.
//!
//! The mutable half — the descriptor sets — is **per frame**. Each
//! frame takes a [`FrameSets`] whose pools are reset before first use,
//! writes every set it needs into it, and releases it only when the
//! submission that referenced it has completed. That is what makes the
//! mirror safe against the things that would otherwise stale it:
//!
//! * **Resource re-creation.** A re-created buffer, image or view can
//!   reuse a raw handle a driver has recycled. A descriptor set that
//!   outlived the resource it was written for would then point at the
//!   wrong memory with no error anywhere. Because no set outlives the
//!   frame it was written in, and the engine creates targets, atlases
//!   and images before the frame's encode begins, a set can never
//!   describe a resource that has since changed identity.
//! * **Atlas growth.** The glyph atlas and the mask texture are
//!   re-created when they grow, within the frame's preparation and
//!   before encode. The frame's [`Stamp`] carries the surface bind
//!   generation, the image-table generation and the mask-texture
//!   generation; [`Mirror::begin_frame`] records it and every write
//!   checks it. A stamp that moves mid-frame is a bug in the caller,
//!   not a case to recover from, and panics.
//! * **Device loss.** Every Vulkan call the mirror makes is checked. A
//!   `VK_ERROR_DEVICE_LOST` surfaces as [`MirrorError::Vulkan`] and
//!   the epoch's encode fails; the mirror is dropped with the renderer
//!   that owns it. Nothing is retried and nothing is reused.
//!
//! Nothing in the mirror is ever *silently* reused: a set is either
//! found under a key that includes every handle written into it, or it
//! is allocated and written afresh.
//!
//! The transient attachments are the other owned resource. They are
//! plain Vulkan images, not wgpu textures, because the executor needs
//! two things wgpu cannot express: `VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT`
//! (`gfx-rs/wgpu#10148`) and one dedicated `VkDeviceMemory` per slot, so
//! `vkGetDeviceMemoryCommitment` answers for that slot alone and the
//! report can state committed bytes honestly rather than estimate them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ash::vk;
use rustc_hash::FxHashMap;

use super::Eligible;
use crate::render::shaders::spirv;

use crate::render::bindings::{
    ENGINE_GROUP0, ENGINE_GROUP1, IMAGE_DECODE_BINDING, TILE_INPUT_BINDINGS, TILE_INPUT_GROUP,
    vulkan_slot,
};

/// Attachment slots this executor binds at once, slot zero — the
/// epoch's stored output — included.
///
/// The same closed set the Metal executor offers: `build_tile_spirv.rs`
/// rewrites one module per source arity, and the per-draw attachment
/// location and input-attachment index remapping
/// (`VK_KHR_dynamic_rendering_local_read`) covers every target/source
/// pair below this bound with those two modules.
pub const SLOTS: u8 = 4;

/// The attachment format every slot carries — the engine's working
/// space, extended linear Display P3 in f16.
const FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;

/// The layout attachments hold for the whole of a tile epoch.
///
/// `VK_KHR_dynamic_rendering_local_read` defines
/// `VK_IMAGE_LAYOUT_RENDERING_LOCAL_READ_KHR` as the layout that is
/// simultaneously a colour attachment and an input attachment for local
/// reads, which is exactly what every draw in an epoch does to its own
/// slot. Using it rather than `GENERAL` keeps whatever framebuffer
/// compression the implementation has for attachments.
pub const LOCAL_READ: vk::ImageLayout = vk::ImageLayout::RENDERING_LOCAL_READ_KHR;

/// Descriptor pools per frame beyond which the mirror refuses to grow.
///
/// A frame releases its pools when its submission completes, so the
/// live count is the number of frames in flight. More than this means
/// completions have stopped arriving, which is a fault to report rather
/// than absorb.
const MAX_FRAMES: usize = 8;

/// Sets one descriptor pool holds before the frame chains another.
const SETS_PER_POOL: u32 = 64;

/// Why the mirror could not do what was asked.
#[derive(Debug)]
pub enum MirrorError {
    /// The engine's device is not a Vulkan device.
    NotVulkan,
    /// A Vulkan call failed; the `&'static str` names the call.
    Vulkan(&'static str, vk::Result),
    /// None of the device's lazily allocated memory types admits the
    /// slot image, so the executor could not keep it off device memory.
    NoLazyMemory,
    /// Submission completions stopped arriving and the per-frame
    /// descriptor pools could not be recycled.
    PoolsExhausted,
}

impl std::fmt::Display for MirrorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotVulkan => write!(f, "the engine device is not Vulkan"),
            Self::Vulkan(call, result) => write!(f, "{call} failed: {result:?}"),
            Self::NoLazyMemory => {
                write!(
                    f,
                    "no lazily allocated memory type admits a tile attachment"
                )
            }
            Self::PoolsExhausted => write!(f, "tile descriptor pools exhausted"),
        }
    }
}

impl std::error::Error for MirrorError {}

/// The resource identities a frame's descriptor sets are written
/// against. Recorded when the frame opens and checked at every write:
/// the engine finishes creating targets, images and atlases before
/// encode, so this cannot move within a frame.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[expect(
    clippy::struct_field_names,
    reason = "each field is a generation counter, and `gen` is what names one"
)]
pub struct Stamp {
    /// The surface's bind generation — bumped whenever its targets are
    /// re-created.
    pub bind_gen: u64,
    /// The image table's generation — bumped whenever a registered
    /// image, bitmap or shader texture is installed or replaced.
    pub images_gen: u64,
    /// The atlas mask texture's generation.
    pub mask_gen: u64,
}

/// One transient attachment slot: its own image, its own dedicated
/// lazily allocated memory, and its view.
///
/// Dedicated rather than suballocated so that
/// `vkGetDeviceMemoryCommitment` on [`Slot::memory`] answers for this
/// slot and nothing else.
#[derive(Debug)]
pub struct Slot {
    image: vk::Image,
    memory: vk::DeviceMemory,
    view: vk::ImageView,
    /// Bytes the allocation reserves — what the slot would have cost in
    /// device memory had it not stayed on chip.
    size: u64,
}

impl Slot {
    /// The slot's view, the operand of both the rendering attachment
    /// and the input attachment descriptor.
    #[must_use]
    pub const fn view(&self) -> vk::ImageView {
        self.view
    }

    /// The slot's image, the operand of its layout barrier.
    #[must_use]
    pub const fn image(&self) -> vk::Image {
        self.image
    }
}

/// The attachments allocated at one size, and whether this frame used
/// them.
#[derive(Default)]
struct SizeEntry {
    slots: Vec<Slot>,
    used: bool,
}

/// One frame's descriptor sets and the pools they came from.
struct FrameSets {
    /// Pools chained as the frame needs more sets than one holds.
    pools: Vec<vk::DescriptorPool>,
    /// Sets written this frame, keyed by everything written into them.
    sets: FxHashMap<SetKey, vk::DescriptorSet>,
    /// Sets already allocated from the last pool.
    allocated: u32,
    /// Set by the submission-completion callback the renderer registers
    /// once the frame's command buffers have retired.
    idle: Arc<AtomicBool>,
}

/// Resources released by the renderer's submission-completion callback.
pub struct Completion {
    device: ash::Device,
    idle: Arc<AtomicBool>,
    retired: Vec<Slot>,
}

impl Completion {
    /// Releases attachments and descriptor pools after their submission completes.
    pub fn complete(self) {
        for slot in self.retired {
            // SAFETY: the queue invokes this only after the submission covering
            // every use of these retired attachments has completed.
            unsafe { slot.destroy(&self.device) };
        }
        self.idle.store(true, Ordering::Release);
    }
}

/// What a descriptor set was written for. Every handle written into a
/// set appears here, so a hit is a set that describes exactly the same
/// resources.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum SetKey {
    /// Group 0: globals, instances, stops, atlas, image transfer table.
    Engine(u64, u64, u64, u64, u64),
    /// Group 1: source, backdrop, image, mask views. The tile path
    /// never binds source or backdrop — they read as attachments — so
    /// only the last two vary, but the key carries all four to stay
    /// honest about what the set holds.
    Sampled(u64, u64, u64, u64),
    /// Group 2: the draw's own slot view and its source slot view.
    Input(u64, u64),
}

/// A graphics pipeline's identity: the same key the Metal executor
/// uses, because the two executors select among the same closed set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct PipeKey {
    /// The slot the draw writes.
    pub target: u8,
    /// The slot the draw samples; equal to `target` for a self-sourced
    /// draw, which takes the second module.
    pub source: u8,
    /// `0` simple, `1` shadow, `2` full, `3` clear, `4` long gradient.
    pub variant: u8,
    /// Replace rather than source-over.
    pub replace: bool,
    /// Attachments the epoch binds.
    pub slots: u8,
}

/// Whether `vkCmdBeginRendering` is reached as core 1.3 or through
/// `VK_KHR_dynamic_rendering`.
enum Rendering {
    Core,
    Khr(ash::khr::dynamic_rendering::Device),
}

/// The executor's mirror of the engine's bind groups, pipeline layout
/// and pipelines, plus the transient attachments it owns.
///
/// See the module documentation for what invalidates each half.
pub struct Mirror {
    device: ash::Device,
    rendering: Rendering,
    local_read: ash::khr::dynamic_rendering_local_read::Device,
    /// The eligibility the mirror was built on, which carries the
    /// device's lazily allocated memory types.
    eligible: Eligible,
    /// The tile modules, `[family][self]` with families short, long
    /// gradient, image source, clipped glyph, span composite and simple (see [`family`]):
    /// `engine_tile{,_long,_image}.spv` for a draw sourcing another slot,
    /// `engine_tile{,_long,_image}_self.spv` for a draw sourcing its own
    /// slot.
    modules: [[vk::ShaderModule; 2]; 6],
    set_layouts: [vk::DescriptorSetLayout; 3],
    layout: vk::PipelineLayout,
    cache: vk::PipelineCache,
    cache_path: Option<std::path::PathBuf>,
    pipelines: FxHashMap<PipeKey, vk::Pipeline>,
    attachments: FxHashMap<(u32, u32), SizeEntry>,
    frames: Vec<FrameSets>,
    active: Option<usize>,
    stamp: Option<Stamp>,
    retired: Vec<Slot>,
}

impl std::fmt::Debug for Mirror {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mirror")
            .field("pipelines", &self.pipelines.len())
            .field("attachment sizes", &self.attachments.len())
            .field("frames", &self.frames.len())
            .finish_non_exhaustive()
    }
}

/// Reads a SPIR-V blob compiled into the engine into the word slice
/// `vkCreateShaderModule` takes.
fn words(bytes: &'static [u8]) -> Vec<u32> {
    let mut cursor = std::io::Cursor::new(bytes);
    ash::util::read_spv(&mut cursor).expect("engine tile SPIR-V is emitted by build.rs")
}

impl Mirror {
    /// Builds the mirror on the engine's own Vulkan device, which
    /// `eligible` says can run an epoch.
    ///
    /// # Errors
    /// [`MirrorError`] when the device is not Vulkan or when any Vulkan
    /// object fails to create.
    pub fn new(
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        eligible: Eligible,
        cache_path: Option<&std::path::Path>,
    ) -> Result<Self, MirrorError> {
        // SAFETY: `as_hal` borrows the adapter for this scope and the
        // callback only reads its instance and physical device.
        let Some(hal_adapter) = (unsafe { adapter.as_hal::<wgpu::hal::vulkan::Api>() }) else {
            return Err(MirrorError::NotVulkan);
        };
        let instance = hal_adapter.shared_instance().raw_instance().clone();
        let physical_device = hal_adapter.raw_physical_device();
        drop(hal_adapter);
        // SAFETY: `physical_device` belongs to `instance`, which is
        // live; the query writes only its own return value.
        let properties = unsafe { instance.get_physical_device_properties(physical_device) };
        let api_version = properties.api_version;
        // SAFETY: `as_hal` borrows the device for this scope; the
        // callback clones the `ash::Device`, which is reference counted
        // by the `wgpu::Device` the caller holds for the mirror's life.
        let Some(raw) = (unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }) else {
            return Err(MirrorError::NotVulkan);
        };
        let vk_device = raw.raw_device().clone();
        let has_khr_dynamic_rendering = raw
            .enabled_device_extensions()
            .contains(&ash::khr::dynamic_rendering::NAME);
        drop(raw);

        let rendering = if api_version >= vk::make_api_version(0, 1, 3, 0) {
            Rendering::Core
        } else if has_khr_dynamic_rendering {
            Rendering::Khr(ash::khr::dynamic_rendering::Device::new(
                &instance, &vk_device,
            ))
        } else {
            return Err(MirrorError::Vulkan(
                "vkCmdBeginRendering",
                vk::Result::ERROR_EXTENSION_NOT_PRESENT,
            ));
        };
        let local_read = ash::khr::dynamic_rendering_local_read::Device::new(&instance, &vk_device);

        // Native Vulkan data must not share wgpu's wrapped cache file.
        let cache_path = cache_path.map(|path| path.with_added_extension("vulkan"));
        let data = cache_path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok());
        let data = data
            .as_deref()
            .filter(|data| compatible_cache(data, &properties));
        let mut built = Built::default();
        let result = built.build(&vk_device, data);
        let Ok(()) = result else {
            // SAFETY: `built` holds only objects created on `vk_device`
            // by this call, each destroyed exactly once.
            unsafe { built.destroy(&vk_device) };
            return Err(result.expect_err("checked"));
        };
        Ok(Self {
            device: vk_device,
            rendering,
            local_read,
            eligible,
            modules: built.modules,
            set_layouts: built.set_layouts,
            layout: built.layout,
            cache: built.cache,
            cache_path,
            pipelines: FxHashMap::default(),
            attachments: FxHashMap::default(),
            frames: Vec::new(),
            active: None,
            stamp: None,
            retired: Vec::new(),
        })
    }

    /// The raw device the executor records against.
    #[must_use]
    pub const fn device(&self) -> &ash::Device {
        &self.device
    }

    /// The pipeline layout every tile draw binds its sets to.
    #[must_use]
    pub const fn pipeline_layout(&self) -> vk::PipelineLayout {
        self.layout
    }

    /// Opens a frame: recycles a released set of descriptor pools, or
    /// takes a fresh one, and records the resource identities the
    /// frame's sets will be written against.
    ///
    /// # Errors
    /// [`MirrorError::PoolsExhausted`] when no frame's pools have been
    /// released and the cap is reached, and [`MirrorError::Vulkan`]
    /// when a pool cannot be reset or created.
    pub fn begin_frame(&mut self, stamp: Stamp) -> Result<(), MirrorError> {
        if let Some(index) = self.active.take() {
            // The previous frame never closed, so its encode failed
            // before the surface submitted. Its command buffers were
            // dropped unsubmitted and nothing ever referenced its sets,
            // so its pools are free again immediately — the release the
            // submission would have given them.
            self.frames[index].idle.store(true, Ordering::Release);
        }
        let free = self
            .frames
            .iter()
            .position(|frame| frame.idle.load(Ordering::Acquire));
        let index = if let Some(index) = free {
            let frame = &mut self.frames[index];
            for pool in &frame.pools {
                // SAFETY: `pool` was created on `self.device` by
                // this mirror, and `idle` says every command buffer
                // that referenced its sets has retired.
                unsafe {
                    self.device
                        .reset_descriptor_pool(*pool, vk::DescriptorPoolResetFlags::default())
                }
                .map_err(|e| MirrorError::Vulkan("vkResetDescriptorPool", e))?;
            }
            frame.sets.clear();
            frame.allocated = 0;
            frame.idle.store(false, Ordering::Release);
            index
        } else {
            if self.frames.len() >= MAX_FRAMES {
                return Err(MirrorError::PoolsExhausted);
            }
            let pool = self.create_pool()?;
            self.frames.push(FrameSets {
                pools: vec![pool],
                sets: FxHashMap::default(),
                allocated: 0,
                idle: Arc::new(AtomicBool::new(false)),
            });
            self.frames.len() - 1
        };
        self.active = Some(index);
        self.stamp = Some(stamp);
        Ok(())
    }

    /// Closes the frame and transfers retired attachments to its completion
    /// callback, which also releases this frame's descriptor pools.
    ///
    /// Returns `None` when the frame recorded no tile epoch.
    pub fn end_frame(&mut self) -> Option<Completion> {
        self.stamp = None;
        let index = self.active.take()?;
        Some(Completion {
            device: self.device.clone(),
            idle: Arc::clone(&self.frames[index].idle),
            retired: std::mem::take(&mut self.retired),
        })
    }

    /// Checks the frame's resource identities have not moved under the
    /// sets it has already written.
    fn check(&self, stamp: Stamp) {
        assert_eq!(
            self.stamp,
            Some(stamp),
            "the engine re-created a resource during a frame's tile encode"
        );
    }

    /// The transient attachments for an epoch of `count` temporary
    /// slots at `size`, created on first use of that size.
    ///
    /// # Errors
    /// [`MirrorError`] when an image, its memory or its view fails.
    pub fn slots(&mut self, size: (u32, u32), count: u8) -> Result<&[Slot], MirrorError> {
        let want = usize::from(count);
        {
            let entry = self.attachments.entry(size).or_default();
            entry.used = true;
        }
        while self.attachments[&size].slots.len() < want {
            let slot = self.create_slot(size)?;
            self.attachments
                .get_mut(&size)
                .expect("inserted above")
                .slots
                .push(slot);
        }
        Ok(&self.attachments[&size].slots[..want])
    }

    /// The committed bytes of this frame's transient attachments, read
    /// from the driver with `vkGetDeviceMemoryCommitment` rather than
    /// estimated.
    ///
    /// Lazily allocated memory commits only the tile backing the
    /// implementation actually needed, so a device that keeps every
    /// slot on chip reports zero and one that had to spill reports what
    /// it spilled. Sizes this frame did not use are excluded.
    #[must_use]
    pub fn committed_bytes(&self) -> u64 {
        self.attachments
            .values()
            .filter(|entry| entry.used)
            .flat_map(|entry| entry.slots.iter())
            // SAFETY: each `slot.memory` was allocated on
            // `self.device` by this mirror and is live.
            .map(|slot| unsafe { self.device.get_device_memory_commitment(slot.memory) })
            .sum()
    }

    /// The bytes this frame's transient attachments would have cost in
    /// device memory had the implementation not kept them on chip —
    /// the Vulkan counterpart of Metal's memoryless accounting (#2547).
    #[must_use]
    pub fn memoryless_bytes(&self) -> u64 {
        self.attachments
            .values()
            .filter(|entry| entry.used)
            .flat_map(|entry| entry.slots.iter())
            .map(|slot| slot.size)
            .sum()
    }

    /// Every live transient attachment's reserved bytes, this frame's
    /// and the sizes a previous frame left allocated.
    ///
    /// This is the steady-state figure `Engine::memory` reports, so it
    /// cannot be scoped to a frame the way [`Self::memoryless_bytes`]
    /// is: between frames nothing is "used", and the allocations are
    /// still there.
    #[must_use]
    pub fn reserved_bytes(&self) -> u64 {
        self.attachments
            .values()
            .flat_map(|entry| entry.slots.iter())
            .map(|slot| slot.size)
            .sum()
    }

    /// What the driver reports committed across every live transient
    /// attachment — the device memory [`Self::reserved_bytes`] actually
    /// cost.
    #[must_use]
    pub fn committed_total_bytes(&self) -> u64 {
        self.attachments
            .values()
            .flat_map(|entry| entry.slots.iter())
            // SAFETY: each `slot.memory` was allocated on
            // `self.device` by this mirror and is live.
            .map(|slot| unsafe { self.device.get_device_memory_commitment(slot.memory) })
            .sum()
    }

    /// Retires attachment sizes this frame did not use, so animated
    /// bounds do not keep every historical size alive. The images are
    /// destroyed by this frame's submission-completion callback, which
    /// covers every earlier submission that referenced them.
    pub fn finish_frame(&mut self) {
        let mut dropped = Vec::new();
        self.attachments.retain(|_, entry| {
            if std::mem::take(&mut entry.used) {
                true
            } else {
                dropped.append(&mut entry.slots);
                false
            }
        });
        self.retired.extend(dropped);
    }

    /// Creates one transient attachment at `size`: an image with the
    /// colour, input and transient usages, its own lazily allocated
    /// dedicated memory, and its view.
    fn create_slot(&self, size: (u32, u32)) -> Result<Slot, MirrorError> {
        let info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(FORMAT)
            .extent(vk::Extent3D {
                width: size.0,
                height: size.1,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(
                vk::ImageUsageFlags::COLOR_ATTACHMENT
                    | vk::ImageUsageFlags::INPUT_ATTACHMENT
                    | vk::ImageUsageFlags::TRANSIENT_ATTACHMENT,
            )
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);
        // SAFETY: `self.device` is live and `info` describes a plain 2D
        // image referencing nothing external.
        let image = unsafe { self.device.create_image(&info, None) }
            .map_err(|e| MirrorError::Vulkan("vkCreateImage", e))?;
        let finish = |slot: Result<Slot, MirrorError>| match slot {
            Ok(slot) => Ok(slot),
            Err(error) => {
                // SAFETY: `image` was created just above on
                // `self.device` and is destroyed once, on this path.
                unsafe { self.device.destroy_image(image, None) };
                Err(error)
            }
        };
        // SAFETY: `image` was created above on `self.device`; a pure
        // query.
        let requirements = unsafe { self.device.get_image_memory_requirements(image) };
        let Some(index) = self
            .eligible
            .lazy_memory_type(requirements.memory_type_bits)
        else {
            return finish(Err(MirrorError::NoLazyMemory));
        };
        // A dedicated allocation is what makes
        // `vkGetDeviceMemoryCommitment` answer for this slot alone:
        // with a suballocating allocator the commitment would cover
        // every image sharing the block.
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let allocate = vk::MemoryAllocateInfo::default()
            .allocation_size(requirements.size)
            .memory_type_index(index)
            .push_next(&mut dedicated);
        // SAFETY: `allocate` names a memory type the requirements
        // admit, and the dedicated chain names the image being bound.
        let memory = match unsafe { self.device.allocate_memory(&allocate, None) } {
            Ok(memory) => memory,
            Err(e) => return finish(Err(MirrorError::Vulkan("vkAllocateMemory", e))),
        };
        let finish = |slot: Result<Slot, MirrorError>| match slot {
            Ok(slot) => Ok(slot),
            Err(error) => {
                // SAFETY: both objects were created above on
                // `self.device` and are torn down once, here.
                unsafe {
                    self.device.free_memory(memory, None);
                    self.device.destroy_image(image, None);
                }
                Err(error)
            }
        };
        // SAFETY: `image` and `memory` were created above on
        // `self.device`; offset zero binds the whole requirement.
        if let Err(e) = unsafe { self.device.bind_image_memory(image, memory, 0) } {
            return finish(Err(MirrorError::Vulkan("vkBindImageMemory", e)));
        }
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_2D)
            .format(FORMAT)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });
        // SAFETY: `image` is live on `self.device` and the view covers
        // its only mip and layer.
        let view = match unsafe { self.device.create_image_view(&view_info, None) } {
            Ok(view) => view,
            Err(e) => return finish(Err(MirrorError::Vulkan("vkCreateImageView", e))),
        };
        Ok(Slot {
            image,
            memory,
            view,
            size: requirements.size,
        })
    }

    /// Creates one descriptor pool sized for [`SETS_PER_POOL`] sets of
    /// the three mirrored layouts.
    fn create_pool(&self) -> Result<vk::DescriptorPool, MirrorError> {
        let sizes = [
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC,
                descriptor_count: SETS_PER_POOL,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::STORAGE_BUFFER,
                descriptor_count: 3 * SETS_PER_POOL,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::SAMPLED_IMAGE,
                descriptor_count: 4 * SETS_PER_POOL,
            },
            vk::DescriptorPoolSize {
                ty: vk::DescriptorType::INPUT_ATTACHMENT,
                descriptor_count: 2 * SETS_PER_POOL,
            },
        ];
        // SAFETY: `self.device` is live and the sizes are constants
        // matching the three mirrored layouts.
        unsafe {
            self.device.create_descriptor_pool(
                &vk::DescriptorPoolCreateInfo::default()
                    .max_sets(SETS_PER_POOL)
                    .pool_sizes(&sizes),
                None,
            )
        }
        .map_err(|e| MirrorError::Vulkan("vkCreateDescriptorPool", e))
    }

    /// Allocates one set of `layout` from the open frame, chaining a
    /// new pool when the current one is full.
    fn allocate(
        &mut self,
        layout: vk::DescriptorSetLayout,
    ) -> Result<vk::DescriptorSet, MirrorError> {
        let index = self.active.expect("a tile frame is open");
        if self.frames[index].allocated >= SETS_PER_POOL {
            let pool = self.create_pool()?;
            let frame = &mut self.frames[index];
            frame.pools.push(pool);
            frame.allocated = 0;
        }
        let frame = &mut self.frames[index];
        let pool = *frame.pools.last().expect("a frame holds a pool");
        let layouts = [layout];
        // SAFETY: `pool` was created by this mirror on `self.device`
        // with room for this set, and `layout` is one of its own.
        let sets = unsafe {
            self.device.allocate_descriptor_sets(
                &vk::DescriptorSetAllocateInfo::default()
                    .descriptor_pool(pool)
                    .set_layouts(&layouts),
            )
        }
        .map_err(|e| MirrorError::Vulkan("vkAllocateDescriptorSets", e))?;
        frame.allocated += 1;
        Ok(sets[0])
    }

    /// The group-0 set over the engine's buffers and atlas, written
    /// once per frame per identity.
    ///
    /// # Errors
    /// [`MirrorError`] when allocation fails.
    ///
    /// # Panics
    /// When the frame's resource identities moved mid-encode.
    pub fn engine_set(
        &mut self,
        stamp: Stamp,
        globals: vk::Buffer,
        instances: vk::Buffer,
        stops: vk::Buffer,
        atlas: vk::ImageView,
        image_decode: vk::Buffer,
    ) -> Result<vk::DescriptorSet, MirrorError> {
        self.check(stamp);
        let key = SetKey::Engine(
            raw(globals),
            raw(instances),
            raw(stops),
            raw_view(atlas),
            raw(image_decode),
        );
        if let Some(set) = self.lookup(key) {
            return Ok(set);
        }
        let set = self.allocate(self.set_layouts[0])?;
        let buffers = [
            vk::DescriptorBufferInfo {
                buffer: globals,
                offset: 0,
                range: std::mem::size_of::<crate::render::instance::Globals>() as u64,
            },
            vk::DescriptorBufferInfo {
                buffer: instances,
                offset: 0,
                range: vk::WHOLE_SIZE,
            },
            vk::DescriptorBufferInfo {
                buffer: stops,
                offset: 0,
                range: vk::WHOLE_SIZE,
            },
            vk::DescriptorBufferInfo {
                buffer: image_decode,
                offset: 0,
                range: vk::WHOLE_SIZE,
            },
        ];
        let image = [vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: atlas,
            image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        }];
        // SAFETY: `set` was allocated from this frame's pool against
        // layout 0, and each write matches that layout's binding type
        // and references a live resource.
        unsafe {
            self.device.update_descriptor_sets(
                &[
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(0)
                        .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                        .buffer_info(std::slice::from_ref(&buffers[0])),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(1)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(std::slice::from_ref(&buffers[1])),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(2)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(std::slice::from_ref(&buffers[2])),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(3)
                        .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                        .image_info(&image),
                    vk::WriteDescriptorSet::default()
                        .dst_set(set)
                        .dst_binding(IMAGE_DECODE_BINDING)
                        .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                        .buffer_info(std::slice::from_ref(&buffers[3])),
                ],
                &[],
            );
        }
        self.insert(key, set);
        Ok(set)
    }

    /// The group-1 set over a draw's sampled image and clip mask. The
    /// tile path never binds source or backdrop views — those are read
    /// as attachments — so both take `dummy`.
    ///
    /// # Errors
    /// [`MirrorError`] when allocation fails.
    ///
    /// # Panics
    /// When the frame's resource identities moved mid-encode.
    pub fn sampled_set(
        &mut self,
        stamp: Stamp,
        dummy: vk::ImageView,
        image: Option<vk::ImageView>,
        mask: Option<vk::ImageView>,
    ) -> Result<vk::DescriptorSet, MirrorError> {
        self.check(stamp);
        let image = image.unwrap_or(dummy);
        let mask = mask.unwrap_or(dummy);
        let key = SetKey::Sampled(
            raw_view(dummy),
            raw_view(dummy),
            raw_view(image),
            raw_view(mask),
        );
        if let Some(set) = self.lookup(key) {
            return Ok(set);
        }
        let set = self.allocate(self.set_layouts[1])?;
        let views = [dummy, dummy, image, mask];
        let infos: Vec<vk::DescriptorImageInfo> = views
            .iter()
            .map(|view| vk::DescriptorImageInfo {
                sampler: vk::Sampler::null(),
                image_view: *view,
                image_layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
            })
            .collect();
        let writes: Vec<vk::WriteDescriptorSet<'_>> = infos
            .iter()
            .enumerate()
            .map(|(binding, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(u32::try_from(binding).expect("four bindings"))
                    .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                    .image_info(std::slice::from_ref(info))
            })
            .collect();
        // SAFETY: `set` was allocated against layout 1 and every write
        // matches one of its sampled-image bindings with a live view.
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        self.insert(key, set);
        Ok(set)
    }

    /// The group-2 set over a draw's two input attachments: its own
    /// slot at index 0 and its source slot at index 1. A self-sourced
    /// draw passes the same view twice and takes the module whose
    /// reads are both at index 0.
    ///
    /// # Errors
    /// [`MirrorError`] when allocation fails.
    ///
    /// # Panics
    /// When the frame's resource identities moved mid-encode.
    pub fn input_set(
        &mut self,
        stamp: Stamp,
        own: vk::ImageView,
        source: vk::ImageView,
    ) -> Result<vk::DescriptorSet, MirrorError> {
        self.check(stamp);
        let key = SetKey::Input(raw_view(own), raw_view(source));
        if let Some(set) = self.lookup(key) {
            return Ok(set);
        }
        let set = self.allocate(self.set_layouts[2])?;
        let infos = [own, source].map(|view| vk::DescriptorImageInfo {
            sampler: vk::Sampler::null(),
            image_view: view,
            image_layout: LOCAL_READ,
        });
        let writes: Vec<vk::WriteDescriptorSet<'_>> = TILE_INPUT_BINDINGS
            .iter()
            .zip(&infos)
            .map(|(binding, info)| {
                vk::WriteDescriptorSet::default()
                    .dst_set(set)
                    .dst_binding(*binding)
                    .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                    .image_info(std::slice::from_ref(info))
            })
            .collect();
        // SAFETY: `set` was allocated against layout 2 and both writes
        // match its input-attachment bindings with live attachment
        // views in the layout the descriptor declares.
        unsafe { self.device.update_descriptor_sets(&writes, &[]) };
        self.insert(key, set);
        Ok(set)
    }

    /// The set written under `key` this frame, if any.
    fn lookup(&self, key: SetKey) -> Option<vk::DescriptorSet> {
        let index = self.active.expect("a tile frame is open");
        self.frames[index].sets.get(&key).copied()
    }

    /// Records `set` as this frame's set for `key`.
    fn insert(&mut self, key: SetKey, set: vk::DescriptorSet) {
        let index = self.active.expect("a tile frame is open");
        self.frames[index].sets.insert(key, set);
    }

    /// The pipeline for `key`, created on first use. `created` is set
    /// when this call built it — the attribution report's first-use
    /// event (#2547).
    ///
    /// # Errors
    /// [`MirrorError::Vulkan`] when pipeline creation fails.
    pub fn pipeline(
        &mut self,
        key: PipeKey,
        created: &mut bool,
    ) -> Result<vk::Pipeline, MirrorError> {
        if let Some(&pipeline) = self.pipelines.get(&key) {
            return Ok(pipeline);
        }
        *created = true;
        let pipeline = self.create_pipeline(key)?;
        self.pipelines.insert(key, pipeline);
        if let Some(path) = &self.cache_path {
            // SAFETY: the cache belongs to this live device and all access
            // is serialized on the renderer thread.
            match unsafe { self.device.get_pipeline_cache_data(self.cache) } {
                Ok(data) => {
                    if let Err(error) = crate::render::write_pipeline_cache(path, &data) {
                        tracing::debug!(?path, %error, "could not persist Vulkan pipeline cache");
                    }
                }
                Err(error) => tracing::debug!(%error, "could not read Vulkan pipeline cache"),
            }
        }
        Ok(pipeline)
    }

    /// Builds one tile pipeline.
    ///
    /// The pipeline writes shader location 0 and the per-draw
    /// `VkRenderingAttachmentLocationInfo` points that location at the
    /// draw's target slot; the matching
    /// `VkRenderingInputAttachmentIndexInfo` names the draw's own slot
    /// as input index 0 and its source as index 1. Both structures are
    /// also chained into the pipeline, which is what makes the dynamic
    /// state the executor sets legal for it.
    ///
    /// Blend and write mask are indexed by attachment, exactly as the
    /// Metal executor's colour target array is: the target slot blends
    /// source-over (or replaces) and writes all channels, and every
    /// other slot writes none.
    fn create_pipeline(&self, key: PipeKey) -> Result<vk::Pipeline, MirrorError> {
        let module = self.modules[family(key.variant)][usize::from(key.source == key.target)];
        let fragment_entry = match key.variant {
            0 => c"tile_simple",
            1 => c"tile_shadow",
            2 | 4 | 5 | 6 | 7 | 8 => c"tile_full",
            3 => c"tile_clear",
            _ => unreachable!("closed tile shader set"),
        };
        let vertex_entry = if key.variant == 3 {
            c"tile_clear_vertex"
        } else {
            c"vs_main"
        };
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(module)
                .name(vertex_entry),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(module)
                .name(fragment_entry),
        ];
        let vertex = vk::PipelineVertexInputStateCreateInfo::default();
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewports = [vk::Viewport::default()];
        let scissors = [vk::Rect2D::default()];
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewports(&viewports)
            .scissors(&scissors);
        let raster = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let msaa = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let attachments: Vec<vk::PipelineColorBlendAttachmentState> = (0..key.slots)
            .map(|slot| {
                let state = vk::PipelineColorBlendAttachmentState::default();
                if slot != key.target {
                    return state.color_write_mask(vk::ColorComponentFlags::empty());
                }
                let state = state.color_write_mask(vk::ColorComponentFlags::RGBA);
                if key.replace {
                    state
                } else {
                    state
                        .blend_enable(true)
                        .src_color_blend_factor(vk::BlendFactor::ONE)
                        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .color_blend_op(vk::BlendOp::ADD)
                        .src_alpha_blend_factor(vk::BlendFactor::ONE)
                        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
                        .alpha_blend_op(vk::BlendOp::ADD)
                }
            })
            .collect();
        // Rasterization-order attachment access is the guarantee that
        // makes a draw's read of its own slot see the fragments of
        // earlier draws — and of earlier primitives of the same draw —
        // at the same pixel, which is Metal's raster-order group.
        let blend = vk::PipelineColorBlendStateCreateInfo::default()
            .flags(
                vk::PipelineColorBlendStateCreateFlags::RASTERIZATION_ORDER_ATTACHMENT_ACCESS_EXT,
            )
            .attachments(&attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let formats = vec![FORMAT; usize::from(key.slots)];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
        let locations = attachment_locations(key.slots, key.target);
        let mut location_info = vk::RenderingAttachmentLocationInfoKHR::default()
            .color_attachment_locations(&locations);
        let indices = input_indices(key.slots, key.target, key.source);
        let mut index_info = vk::RenderingInputAttachmentIndexInfoKHR::default()
            .color_attachment_input_indices(&indices);
        let info = vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&raster)
            .multisample_state(&msaa)
            .color_blend_state(&blend)
            .dynamic_state(&dynamic)
            .layout(self.layout)
            .push_next(&mut rendering)
            .push_next(&mut location_info)
            .push_next(&mut index_info);
        // SAFETY: `self.device` is live; the create info references
        // only this mirror's own module, layout and cache, and the
        // state structures built above live across the call.
        let pipelines = unsafe {
            self.device
                .create_graphics_pipelines(self.cache, std::slice::from_ref(&info), None)
        }
        .map_err(|(_, e)| MirrorError::Vulkan("vkCreateGraphicsPipelines", e))?;
        Ok(pipelines[0])
    }

    /// `vkCmdBeginRendering` through whichever of core 1.3 or
    /// `VK_KHR_dynamic_rendering` the device offers.
    ///
    /// # Safety
    /// `cb` must be a recording command buffer on this device with no
    /// rendering open, and `info` must describe live attachments.
    pub unsafe fn begin_rendering(&self, cb: vk::CommandBuffer, info: &vk::RenderingInfo<'_>) {
        match &self.rendering {
            // SAFETY: the caller's contract.
            Rendering::Core => unsafe { self.device.cmd_begin_rendering(cb, info) },
            // SAFETY: the caller's contract.
            Rendering::Khr(khr) => unsafe { khr.cmd_begin_rendering(cb, info) },
        }
    }

    /// `vkCmdEndRendering`.
    ///
    /// # Safety
    /// `cb` must be recording with rendering open.
    pub unsafe fn end_rendering(&self, cb: vk::CommandBuffer) {
        match &self.rendering {
            // SAFETY: the caller's contract.
            Rendering::Core => unsafe { self.device.cmd_end_rendering(cb) },
            // SAFETY: the caller's contract.
            Rendering::Khr(khr) => unsafe { khr.cmd_end_rendering(cb) },
        }
    }

    /// Points shader location 0 at `target` and input indices 0 and 1
    /// at `target` and `source`, for the draw about to be recorded.
    ///
    /// # Safety
    /// `cb` must be recording inside a rendering begun with the same
    /// attachment count.
    pub unsafe fn remap(&self, cb: vk::CommandBuffer, slots: u8, target: u8, source: u8) {
        let locations = attachment_locations(slots, target);
        let indices = input_indices(slots, target, source);
        let location_info = vk::RenderingAttachmentLocationInfoKHR::default()
            .color_attachment_locations(&locations);
        let index_info = vk::RenderingInputAttachmentIndexInfoKHR::default()
            .color_attachment_input_indices(&indices);
        // ash 0.38 generates no wrapper methods for this extension, so
        // the entry points are called through the loaded pointers.
        // SAFETY: the caller's contract; both structures and the slices
        // they point at live across their own call.
        unsafe {
            (self
                .local_read
                .fp()
                .cmd_set_rendering_attachment_locations_khr)(
                cb,
                std::ptr::from_ref(&location_info),
            );
            (self
                .local_read
                .fp()
                .cmd_set_rendering_input_attachment_indices_khr)(
                cb,
                std::ptr::from_ref(&index_info),
            );
        }
    }

    /// Destroys every object the mirror owns.
    ///
    /// # Safety
    /// Every submission that referenced them must have completed.
    unsafe fn destroy(&mut self) {
        // SAFETY: every object below was created by this mirror on
        // `self.device`, is drained so it is destroyed exactly once,
        // and the caller guarantees nothing in flight refers to it.
        unsafe {
            for (_, pipeline) in self.pipelines.drain() {
                self.device.destroy_pipeline(pipeline, None);
            }
            for (_, entry) in self.attachments.drain() {
                for slot in entry.slots {
                    slot.destroy(&self.device);
                }
            }
            for slot in self.retired.drain(..) {
                slot.destroy(&self.device);
            }
            for frame in self.frames.drain(..) {
                for pool in frame.pools {
                    self.device.destroy_descriptor_pool(pool, None);
                }
            }
            self.device.destroy_pipeline_cache(self.cache, None);
            self.device.destroy_pipeline_layout(self.layout, None);
            for layout in self.set_layouts {
                self.device.destroy_descriptor_set_layout(layout, None);
            }
            for module in self.modules.iter().flatten() {
                self.device.destroy_shader_module(*module, None);
            }
        }
    }
}

impl Drop for Mirror {
    fn drop(&mut self) {
        // The renderer drops the mirror with the device it was built
        // on, after wgpu has waited for every submission; a device that
        // is already lost accepts the destruction regardless.
        // SAFETY: as above — nothing in flight refers to these objects.
        unsafe { self.destroy() };
    }
}

impl Slot {
    /// # Safety
    /// `device` must be the device the slot was created on and no
    /// submission referencing it may be in flight.
    unsafe fn destroy(self, device: &ash::Device) {
        // SAFETY: the caller's contract; each object is destroyed once
        // because `self` is consumed.
        unsafe {
            device.destroy_image_view(self.view, None);
            device.destroy_image(self.image, None);
            device.free_memory(self.memory, None);
        }
    }
}

/// `pColorAttachmentLocations`: attachment `target` carries the
/// fragment's only output, location 0; every other attachment is
/// unused by the shader and only present so the epoch keeps its slots
/// bound across the whole rendering.
fn attachment_locations(slots: u8, target: u8) -> Vec<u32> {
    (0..slots)
        .map(|slot| {
            if slot == target {
                0
            } else {
                vk::ATTACHMENT_UNUSED
            }
        })
        .collect()
}

/// `pColorAttachmentInputIndices`: the draw reads its own slot at input
/// index 0 and its source at index 1. A self-sourced draw maps only
/// index 0, and `engine_tile_self.spv` reads index 0 for both operands.
fn input_indices(slots: u8, target: u8, source: u8) -> Vec<u32> {
    (0..slots)
        .map(|slot| {
            if slot == target {
                0
            } else if slot == source {
                1
            } else {
                vk::ATTACHMENT_UNUSED
            }
        })
        .collect()
}

/// A buffer handle as the integer the set key stores.
fn raw(buffer: vk::Buffer) -> u64 {
    ash::vk::Handle::as_raw(buffer)
}

/// The tile module family a pipeline key's engine variant draws through:
/// Short paint, long gradients, images, glyphs, composites and simple paint
/// occupy the six module families in that order.
const fn family(variant: u8) -> usize {
    match variant {
        0 => 5,
        4 => 1,
        6 => 2,
        7 => 3,
        8 => 4,
        _ => 0,
    }
}

/// A view handle as the integer the set key stores.
fn raw_view(view: vk::ImageView) -> u64 {
    ash::vk::Handle::as_raw(view)
}

/// The objects [`Mirror::new`] creates before the mirror exists, so a
/// failure part way through can destroy exactly what was made.
#[derive(Default)]
struct Built {
    /// `[family][self]` — see [`Mirror::modules`].
    modules: [[vk::ShaderModule; 2]; 6],
    set_layouts: [vk::DescriptorSetLayout; 3],
    layout: vk::PipelineLayout,
    cache: vk::PipelineCache,
}

impl Built {
    /// Creates the mirror's immutable objects on `device`.
    fn build(&mut self, device: &ash::Device, data: Option<&[u8]>) -> Result<(), MirrorError> {
        let create_module = |bytes| {
            let code = words(bytes);
            // SAFETY: `device` is live and `code` is SPIR-V the build
            // emitted into this binary — valid words by construction.
            unsafe {
                device
                    .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
            }
            .map_err(|e| MirrorError::Vulkan("vkCreateShaderModule", e))
        };
        let blobs = [
            spirv::TILE,
            spirv::TILE_SELF,
            spirv::TILE_LONG,
            spirv::TILE_LONG_SELF,
            spirv::TILE_IMAGE,
            spirv::TILE_IMAGE_SELF,
            spirv::TILE_GLYPH,
            spirv::TILE_GLYPH_SELF,
            spirv::TILE_COMPOSITE,
            spirv::TILE_COMPOSITE_SELF,
            spirv::TILE_SIMPLE,
            spirv::TILE_SIMPLE_SELF,
        ];
        for (slot, bytes) in self.modules.iter_mut().flatten().zip(blobs) {
            *slot = create_module(bytes)?;
        }

        let fragment = vk::ShaderStageFlags::FRAGMENT;
        let both = vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::FRAGMENT;
        let sampled = |binding| {
            vk::DescriptorSetLayoutBinding::default()
                .binding(binding)
                .descriptor_type(vk::DescriptorType::SAMPLED_IMAGE)
                .descriptor_count(1)
                .stage_flags(fragment)
        };
        // Group 0 mirrors `ENGINE_GROUP0`: the globals uniform window
        // the dynamic offset selects a pass's slot in, the instance and
        // stop storage buffers, the glyph atlas and the image transfer
        // table. The GPU coverage cell buffer is left out: the tile
        // families are built without the `// cell-buffer` spans, and a
        // GPU-routed range ends the tile epoch at its materialization
        // boundary.
        let group0 = [
            vk::DescriptorSetLayoutBinding::default()
                .binding(0)
                .descriptor_type(vk::DescriptorType::UNIFORM_BUFFER_DYNAMIC)
                .descriptor_count(1)
                .stage_flags(both),
            vk::DescriptorSetLayoutBinding::default()
                .binding(1)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(both),
            vk::DescriptorSetLayoutBinding::default()
                .binding(2)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(fragment),
            sampled(3),
            vk::DescriptorSetLayoutBinding::default()
                .binding(IMAGE_DECODE_BINDING)
                .descriptor_type(vk::DescriptorType::STORAGE_BUFFER)
                .descriptor_count(1)
                .stage_flags(fragment),
        ];
        // Group 1 mirrors `ENGINE_GROUP1`: source, backdrop, image and
        // mask. A tile draw binds the dummy for the first two.
        let group1 = [sampled(0), sampled(1), sampled(2), sampled(3)];
        // Group 2 is the executor's own: the two input attachments
        // `build_tile_spirv.rs` rewrote the attachment reads onto.
        let group2: Vec<vk::DescriptorSetLayoutBinding<'_>> = TILE_INPUT_BINDINGS
            .iter()
            .map(|binding| {
                vk::DescriptorSetLayoutBinding::default()
                    .binding(*binding)
                    .descriptor_type(vk::DescriptorType::INPUT_ATTACHMENT)
                    .descriptor_count(1)
                    .stage_flags(fragment)
            })
            .collect();
        // wgpu-hal binds a WGSL `@binding` at its ordinal position in
        // the group, and `build.rs` emits the SPIR-V decorations
        // through the same rule. The two engine tables are contiguous
        // and in order, so the layouts above can name the bindings
        // directly — but only while that holds, and a table edited out
        // of order must not silently misbind the mirror.
        for (group, layout) in [(ENGINE_GROUP0, &group0[..]), (ENGINE_GROUP1, &group1[..])] {
            for binding in layout {
                assert_eq!(
                    vulkan_slot(group, binding.binding),
                    binding.binding,
                    "the engine binding tables are no longer in ordinal order"
                );
            }
        }
        let tables: [&[vk::DescriptorSetLayoutBinding<'_>]; 3] = [&group0, &group1, &group2];
        for (slot, bindings) in tables.iter().enumerate() {
            // SAFETY: `device` is live and the info points at the
            // bindings slice, which outlives the call.
            self.set_layouts[slot] = unsafe {
                device.create_descriptor_set_layout(
                    &vk::DescriptorSetLayoutCreateInfo::default().bindings(bindings),
                    None,
                )
            }
            .map_err(|e| MirrorError::Vulkan("vkCreateDescriptorSetLayout", e))?;
        }
        debug_assert_eq!(
            TILE_INPUT_GROUP, 2,
            "the rewritten tile SPIR-V reads its input attachments from group 2"
        );
        // SAFETY: `device` is live and the three layouts were created
        // above on it.
        self.layout = unsafe {
            device.create_pipeline_layout(
                &vk::PipelineLayoutCreateInfo::default().set_layouts(&self.set_layouts),
                None,
            )
        }
        .map_err(|e| MirrorError::Vulkan("vkCreatePipelineLayout", e))?;
        self.cache = create_cache(device, data)?;
        Ok(())
    }

    /// Destroys whatever was created before a failure.
    ///
    /// # Safety
    /// `device` must be the device the objects were created on.
    unsafe fn destroy(&mut self, device: &ash::Device) {
        // SAFETY: each handle is either null — which Vulkan accepts and
        // ignores — or an object created on `device` by `build`.
        unsafe {
            device.destroy_pipeline_cache(self.cache, None);
            device.destroy_pipeline_layout(self.layout, None);
            for layout in self.set_layouts {
                device.destroy_descriptor_set_layout(layout, None);
            }
            for module in self.modules.iter().flatten() {
                device.destroy_shader_module(*module, None);
            }
        }
    }
}

/// Vulkan's version-one cache header is four little-endian u32 fields
/// followed by the device's pipeline-cache UUID. Other devices or driver
/// cache versions must start empty.
fn compatible_cache(data: &[u8], properties: &vk::PhysicalDeviceProperties) -> bool {
    let Some(header) = data.get(..32) else {
        return false;
    };
    let word =
        |offset| u32::from_le_bytes(header[offset..offset + 4].try_into().expect("header word"));
    word(0) == 32
        && word(4)
            == u32::try_from(vk::PipelineCacheHeaderVersion::ONE.as_raw())
                .expect("positive version")
        && word(8) == properties.vendor_id
        && word(12) == properties.device_id
        && header[16..32] == properties.pipeline_cache_uuid
}

fn create_cache(
    device: &ash::Device,
    data: Option<&[u8]>,
) -> Result<vk::PipelineCache, MirrorError> {
    // SAFETY: the bytes came from our cache file and their Vulkan
    // header matches this device; the slice outlives creation.
    unsafe {
        device.create_pipeline_cache(
            &vk::PipelineCacheCreateInfo::default().initial_data(data.unwrap_or_default()),
            None,
        )
    }
    .map_err(|e| MirrorError::Vulkan("vkCreatePipelineCache", e))
}
