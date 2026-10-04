//! Vulkan external-frame coverage on lavapipe (issue #166, plan E7).
//!
//! The capability record drives every test: what lavapipe supports runs
//! for real — a frame built on the same device exercises the full
//! native path — and what it cannot do is reported by name, never faked
//! through a copied image.
//!
//! Honest labels on this lavapipe build (`VK_EXT_external_memory_dma_buf`,
//! external-semaphore fd payloads, `VK_EXT_image_drm_format_modifier`,
//! YCbCr sampler conversion and the foreign-family extension are all
//! absent):
//!
//! - `AHardwareBuffer` native import: not applicable (not Android)
//! - dma-buf native import / modifier round trip: unavailable
//! - external semaphore fd import/export: unavailable
//! - foreign queue-family transfer: unavailable
//! - external-format combined-sampler path: unavailable

#![cfg(all(unix, not(target_vendor = "apple")))]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ash::vk;
use ash::vk::Handle as _;
use cherenkov::{Engine, FrameTime, Next};
use rustc_hash::FxHashMap;

use crate::interop::{
    ExternalFrame, FrameColor, OutputAlpha, OutputColor, Presenter, SharedDevice, TextureOutput,
    TextureTarget,
};
use crate::render::external::{
    self,
    vulkan::{self, NativeError, dmabuf},
};
use crate::{Gpu, GpuConfig};

/// A `SharedDevice` on the Vulkan backend plus the native import device,
/// or `None` when no Vulkan adapter exists (every test skips in that case).
fn setup() -> Option<(SharedDevice, vulkan::Device)> {
    let shared = SharedDevice::create(&GpuConfig::default()).ok()?;
    if shared.adapter.get_info().backend != wgpu::Backend::Vulkan {
        return None;
    }
    let device = vulkan::Device::new(&shared).ok()?;
    Some((shared, device))
}

/// The raw handles tests use to act as the producer: allocate and write
/// images, run one-shot barriers, create and signal timeline semaphores.
fn raw(shared: &SharedDevice) -> (ash::Device, vk::Queue, u32) {
    // SAFETY: `shared.device` is a wgpu device whose adapter was checked
    // to be Vulkan in `setup`, so the hal view exists and borrows for the
    // lifetime of `shared`.
    let hal = unsafe { shared.device.as_hal::<wgpu::hal::vulkan::Api>() };
    let hal = hal.as_ref().expect("vulkan device");
    let device = hal.raw_device().clone();
    let queue = hal.raw_queue();
    (device, queue, hal.queue_family_index())
}

/// Runs a single-command-buffer `record` and waits for completion. The
/// producer side of a test may wait on the host — only the engine path
/// must never.
fn run_once(
    dev: &ash::Device,
    queue: vk::Queue,
    family: u32,
    record: impl FnOnce(vk::CommandBuffer),
) {
    // SAFETY: `dev` is a live device handle and `family` is the engine's
    // queue family index — the create info references nothing else.
    let pool = unsafe {
        dev.create_command_pool(
            &vk::CommandPoolCreateInfo::default()
                .queue_family_index(family)
                .flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),
            None,
        )
    }
    .expect("command pool");
    // SAFETY: `pool` was created above on `dev` and outlives the call.
    let buffers = unsafe {
        dev.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .expect("command buffer");
    let cb = buffers[0];
    // SAFETY: `cb` was allocated from `pool`, records once, submits once,
    // and the queue is waited idle before `pool` is destroyed — every
    // handle in the block stays valid and unmatched teardown is impossible.
    unsafe {
        dev.begin_command_buffer(
            cb,
            &vk::CommandBufferBeginInfo::default()
                .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
        )
        .expect("begin");
        record(cb);
        dev.end_command_buffer(cb).expect("end");
        dev.queue_submit(
            queue,
            &[vk::SubmitInfo::default().command_buffers(&[cb])],
            vk::Fence::null(),
        )
        .expect("submit");
        dev.queue_wait_idle(queue).expect("producer wait");
        dev.destroy_command_pool(pool, None);
    }
}

/// A host-created NV12 image standing in for a producer frame: linear
/// tiling and host-visible memory (lavapipe is a UMA driver), one write
/// by the host, per-plane integer views, and the barrier the producer
/// would run before handoff.
#[derive(Clone, Copy)]
struct Nv12 {
    image: vk::Image,
    memory: vk::DeviceMemory,
    y: vk::ImageView,
    uv: vk::ImageView,
    /// The layout the producer left the image in.
    layout: vk::ImageLayout,
    /// The producer's family.
    family: vulkan::QueueFamily,
}

/// Allocates an NV12 image of `extent` and fills it with `luma`/`chroma`
/// (video-range 8-bit). `None` when the driver lacks the format.
#[expect(clippy::too_many_lines)]
fn make_nv12(
    shared: &SharedDevice,
    device: &vulkan::Device,
    extent: (u32, u32),
    luma: u8,
    chroma: (u8, u8),
) -> Option<Nv12> {
    let (dev, queue, family) = raw(shared);
    let (inst, phys) = (&device.shared.instance, device.shared.physical_device);
    // SAFETY: `inst`/`phys` are the engine's own instance and physical
    // device; the call is a query writing into an out struct.
    let props = unsafe {
        inst.get_physical_device_format_properties(phys, vk::Format::G8_B8R8_2PLANE_420_UNORM)
    };
    if !props
        .optimal_tiling_features
        .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
    {
        return None;
    }
    // SAFETY: `dev` is live and the create info is self-contained — an
    // unsupported combination simply fails the call, which `.ok()?` drops.
    let image = unsafe {
        dev.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::G8_B8R8_2PLANE_420_UNORM)
                .extent(vk::Extent3D {
                    width: extent.0,
                    height: extent.1,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .flags(vk::ImageCreateFlags::MUTABLE_FORMAT)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )
    }
    .ok()?;
    // SAFETY: `image` was created above on `dev` — a pure query.
    let reqs = unsafe { dev.get_image_memory_requirements(image) };
    // SAFETY: `inst`/`phys` are the engine's own handles — a pure query.
    let mem_props = unsafe { inst.get_physical_device_memory_properties(phys) };
    let mut memory_type = None;
    for i in 0..mem_props.memory_type_count {
        let supported = reqs.memory_type_bits & (1 << i) != 0;
        let flags = mem_props.memory_types[i as usize].property_flags;
        if supported && flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE) {
            memory_type = Some(i);
            break;
        }
    }
    // SAFETY: `reqs` was just queried for `image` and `memory_type` is a
    // supported index — the allocation matches the image's requirements.
    let memory = unsafe {
        dev.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type?),
            None,
        )
    }
    .ok()?;
    // SAFETY: `image` and `memory` were created above on `dev`; offset 0
    // binds the whole requirement.
    unsafe { dev.bind_image_memory(image, memory, 0) }.ok()?;
    // Write the planes through the host mapping, honouring the subresource
    // layouts the driver reports.
    // SAFETY: `image` is live on `dev`; the call queries one aspect's
    // subresource layout.
    let plane_layout = |aspect: vk::ImageAspectFlags| unsafe {
        dev.get_image_subresource_layout(
            image,
            vk::ImageSubresource {
                aspect_mask: aspect,
                mip_level: 0,
                array_layer: 0,
            },
        )
    };
    let y_layout = plane_layout(vk::ImageAspectFlags::PLANE_0);
    let uv_layout = plane_layout(vk::ImageAspectFlags::PLANE_1);
    let _ = (y_layout, uv_layout);
    // Fill through a staging buffer: the host-visible staging copy is the
    // producer's write, copied per plane by `vkCmdCopyBufferToImage`.
    let staging_data = {
        let (w, h) = (extent.0 as usize, extent.1 as usize);
        let mut y = vec![luma; w * h];
        let mut uv = Vec::with_capacity(w * h / 2);
        for _ in 0..(w / 2) * (h / 2) {
            uv.push(chroma.0);
            uv.push(chroma.1);
        }
        y.append(&mut uv);
        y
    };
    // SAFETY: `dev` is live and the create info sizes itself from the
    // data — nothing external is referenced.
    let staging = unsafe {
        dev.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(staging_data.len() as u64)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC)
                .sharing_mode(vk::SharingMode::EXCLUSIVE),
            None,
        )
    }
    .ok()?;
    // SAFETY: `staging` was created above on `dev` — a pure query.
    let sreqs = unsafe { dev.get_buffer_memory_requirements(staging) };
    let mut smem_type = None;
    for i in 0..mem_props.memory_type_count {
        let flags = mem_props.memory_types[i as usize].property_flags;
        if sreqs.memory_type_bits & (1 << i) != 0
            && flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
        {
            smem_type = Some(i);
            break;
        }
    }
    // SAFETY: `sreqs` was just queried for `staging` and `smem_type` is a
    // supported index — the allocation matches the buffer's requirements.
    let smem = unsafe {
        dev.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(sreqs.size)
                .memory_type_index(smem_type?),
            None,
        )
    }
    .ok()?;
    // SAFETY: `staging`/`smem` were created above and bind at offset 0;
    // `WHOLE_SIZE` maps the whole allocation and the copy writes
    // `staging_data.len()` bytes — no more than the buffer's own size.
    unsafe {
        dev.bind_buffer_memory(staging, smem, 0).ok()?;
        let base = dev
            .map_memory(smem, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            .ok()?;
        base.cast::<u8>()
            .copy_from_nonoverlapping(staging_data.as_ptr(), staging_data.len());
        dev.unmap_memory(smem);
    }
    let (w, h) = extent;
    // SAFETY: `cb` is recording inside `run_once`; `staging`, `image` are
    // live on `dev` and the barrier/copy reference real subresources whose
    // layouts match the transitions recorded here.
    run_once(&dev, queue, family, |cb| unsafe {
        dev.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::empty())
                .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })],
        );
        dev.cmd_copy_buffer_to_image(
            cb,
            staging,
            image,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[
                vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::PLANE_0,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    }),
                vk::BufferImageCopy::default()
                    .buffer_offset(u64::from(w * h))
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::PLANE_1,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: w / 2,
                        height: h / 2,
                        depth: 1,
                    }),
            ],
        );
    });
    // SAFETY: `run_once` waited the queue idle, so no command still
    // references `staging`/`smem`; each is destroyed and freed exactly once.
    unsafe {
        dev.destroy_buffer(staging, None);
        dev.free_memory(smem, None);
    }
    let view = |aspect, format| {
        // SAFETY: `image` is live on `dev`; each call creates a view on a
        // valid aspect/format pair for that image.
        unsafe {
            dev.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(format)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: aspect,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    }),
                None,
            )
        }
        .expect("plane view")
    };
    let (y, uv) = (
        view(vk::ImageAspectFlags::PLANE_0, vk::Format::R8_UINT),
        view(vk::ImageAspectFlags::PLANE_1, vk::Format::R8G8_UINT),
    );
    // The producer's final barrier: the image lands in GENERAL so the
    // acquisition barrier does a real layout change.
    // SAFETY: `cb` is recording inside `run_once` and the barrier
    // references the live `image`.
    run_once(&dev, queue, family, |cb| unsafe {
        dev.cmd_pipeline_barrier(
            cb,
            vk::PipelineStageFlags::HOST,
            vk::PipelineStageFlags::ALL_COMMANDS,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[vk::ImageMemoryBarrier::default()
                .src_access_mask(vk::AccessFlags::HOST_WRITE)
                .dst_access_mask(vk::AccessFlags::empty())
                .old_layout(vk::ImageLayout::UNDEFINED)
                .new_layout(vk::ImageLayout::GENERAL)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(image)
                .subresource_range(vk::ImageSubresourceRange {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    base_mip_level: 0,
                    level_count: 1,
                    base_array_layer: 0,
                    layer_count: 1,
                })],
        );
    });
    Some(Nv12 {
        image,
        memory,
        y,
        uv,
        layout: vk::ImageLayout::GENERAL,
        family: vulkan::QueueFamily::Index(family),
    })
}

/// Fabricates the `Generation` a dmabuf import would have produced for
/// `nv12`, minus the fd: a same-device image with the same plane views,
/// state machine and lifecycle contract.
fn nv12_generation(
    device: &vulkan::Device,
    nv12: Nv12,
    extent: (u32, u32),
    wait: Option<vulkan::Wait>,
    release: Option<vulkan::ReleaseSync>,
) -> Arc<vulkan::Generation> {
    let pool = Some(dmabuf::create_pool(&device.shared, false).expect("pool"));
    Arc::new(vulkan::Generation {
        shared: device.shared.clone(),
        size: extent,
        color: FrameColor::BT709_VIDEO,
        alpha: crate::interop::RgbAlpha::Opaque,
        repr: vulkan::Repr::Planes {
            kind: external::KIND_NV12,
        },
        image: nv12.image,
        views: vulkan::sync::Views::Planes {
            y: nv12.y,
            uv: nv12.uv,
        },
        conv: None,
        rgb_wrap: None,
        bytes: 0,
        producer_layout: nv12.layout,
        producer_family: nv12.family,
        aspects: vk::ImageAspectFlags::COLOR,
        pool,
        sets: std::sync::Mutex::new(FxHashMap::default()),
        state: Arc::new(std::sync::Mutex::new(vulkan::State::Registered)),
        wait: std::sync::Mutex::new(wait),
        resolved_wait: std::sync::Mutex::new(None),
        release_sync: std::sync::Mutex::new(release),
        fence_semaphore: std::sync::Mutex::new(None),
        fence_fd: None,
        release_submitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        leases: AtomicUsize::new(0),
        parts: std::sync::Mutex::new(Some(vulkan::sync::Release {
            image: nv12.image,
            pool,
            views: vec![nv12.y, nv12.uv],
            conv: None,
            memory: vec![nv12.memory],
            semaphores: vec![],
            sync_payload: None,
            fence_semaphore: None,
            submitted_flag: None,
            state: None,
            acquired: false,
            producer_layout: nv12.layout,
            producer_family: nv12.family,
            aspects: vk::ImageAspectFlags::COLOR,
            lease: vulkan::sync::Lease::None,
            fence_fd: None,
            plane_fences: vec![],
        })),
        #[cfg(target_os = "android")]
        plane: None,
        plane_fences: std::sync::Mutex::new(vec![]),
    })
}

/// A same-device single-plane RGB image standing in for a producer RGB
/// frame: `Repr::Rgb` wraps it through `texture_from_raw` +
/// `create_texture_from_hal` with the actual producer state.
#[derive(Clone, Copy)]
struct Rgb {
    image: vk::Image,
    memory: vk::DeviceMemory,
    layout: vk::ImageLayout,
    family: vulkan::QueueFamily,
}

/// Allocates an `R8G8B8A8_UNORM` image of `extent` filled with `rgba`.
#[expect(clippy::too_many_lines)]
fn make_rgb(
    shared: &SharedDevice,
    device: &vulkan::Device,
    extent: (u32, u32),
    rgba: [u8; 4],
) -> Option<Rgb> {
    let (dev, queue, family) = raw(shared);
    let (inst, phys) = (&device.shared.instance, device.shared.physical_device);
    // SAFETY: `dev` is live and the create info is self-contained.
    let image = unsafe {
        dev.create_image(
            &vk::ImageCreateInfo::default()
                .image_type(vk::ImageType::TYPE_2D)
                .format(vk::Format::R8G8B8A8_UNORM)
                .extent(vk::Extent3D {
                    width: extent.0,
                    height: extent.1,
                    depth: 1,
                })
                .mip_levels(1)
                .array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1)
                .tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
                .sharing_mode(vk::SharingMode::EXCLUSIVE)
                .initial_layout(vk::ImageLayout::UNDEFINED),
            None,
        )
    }
    .ok()?;
    // SAFETY: `image` was created above on `dev` — a pure query.
    let reqs = unsafe { dev.get_image_memory_requirements(image) };
    // SAFETY: `inst`/`phys` are the engine's own handles — a pure query.
    let mem_props = unsafe { inst.get_physical_device_memory_properties(phys) };
    let mut memory_type = None;
    for i in 0..mem_props.memory_type_count {
        let flags = mem_props.memory_types[i as usize].property_flags;
        if reqs.memory_type_bits & (1 << i) != 0
            && flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
        {
            memory_type = Some(i);
            break;
        }
    }
    // SAFETY: `reqs` was just queried for `image` and `memory_type` is a
    // supported index — the allocation matches the image's requirements.
    let memory = unsafe {
        dev.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(reqs.size)
                .memory_type_index(memory_type?),
            None,
        )
    }
    .ok()?;
    // SAFETY: `image` and `memory` were created above on `dev`; offset 0
    // binds the whole requirement.
    unsafe { dev.bind_image_memory(image, memory, 0) }.ok()?;
    // The producer writes through a staging buffer, then leaves the image
    // in GENERAL — the engine's acquire barrier transitions from the
    // producer's actual layout, not a declared convenience.
    let pixels = vec![0u8; (extent.0 * extent.1 * 4) as usize]
        .chunks_exact_mut(4)
        .flat_map(|px| {
            px.copy_from_slice(&rgba);
            rgba
        })
        .collect::<Vec<u8>>();
    // SAFETY: `dev` is live and the create info sizes itself from the
    // data — nothing external is referenced.
    let staging = unsafe {
        dev.create_buffer(
            &vk::BufferCreateInfo::default()
                .size(pixels.len() as u64)
                .usage(vk::BufferUsageFlags::TRANSFER_SRC),
            None,
        )
    }
    .ok()?;
    // SAFETY: `staging` was created above on `dev` — a pure query.
    let sreqs = unsafe { dev.get_buffer_memory_requirements(staging) };
    let mut smem_type = None;
    for i in 0..mem_props.memory_type_count {
        let flags = mem_props.memory_types[i as usize].property_flags;
        if sreqs.memory_type_bits & (1 << i) != 0
            && flags.contains(vk::MemoryPropertyFlags::HOST_VISIBLE)
        {
            smem_type = Some(i);
            break;
        }
    }
    // SAFETY: `sreqs` was just queried for `staging` and `smem_type` is a
    // supported index — the allocation matches the buffer's requirements.
    let smem = unsafe {
        dev.allocate_memory(
            &vk::MemoryAllocateInfo::default()
                .allocation_size(sreqs.size)
                .memory_type_index(smem_type?),
            None,
        )
    }
    .ok()?;
    // SAFETY: `staging`/`smem` were created above and bind at offset 0;
    // `WHOLE_SIZE` maps the whole allocation and the copy writes
    // `pixels.len()` bytes — no more than the buffer's own size.
    unsafe {
        dev.bind_buffer_memory(staging, smem, 0).ok()?;
        let base = dev
            .map_memory(smem, 0, vk::WHOLE_SIZE, vk::MemoryMapFlags::empty())
            .ok()?;
        base.cast::<u8>()
            .copy_from_nonoverlapping(pixels.as_ptr(), pixels.len());
        dev.unmap_memory(smem);
    }
    let (w, h) = extent;
    run_once(&dev, queue, family, |cb| {
        // SAFETY: `cb` is recording inside `run_once`; `staging`, `image`
        // are live on `dev` and the barrier/copy reference real
        // subresources whose layouts match the transitions recorded here.
        unsafe {
            dev.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TOP_OF_PIPE,
                vk::PipelineStageFlags::TRANSFER,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })],
            );
            dev.cmd_copy_buffer_to_image(
                cb,
                staging,
                image,
                vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                &[vk::BufferImageCopy::default()
                    .image_subresource(vk::ImageSubresourceLayers {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        mip_level: 0,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .image_extent(vk::Extent3D {
                        width: w,
                        height: h,
                        depth: 1,
                    })],
            );
            // The producer's handoff barrier leaves the image in GENERAL.
            dev.cmd_pipeline_barrier(
                cb,
                vk::PipelineStageFlags::TRANSFER,
                vk::PipelineStageFlags::ALL_COMMANDS,
                vk::DependencyFlags::empty(),
                &[],
                &[],
                &[vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .image(image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })],
            );
        }
    });
    // SAFETY: `run_once` waited the queue idle, so no command still
    // references `staging`/`smem`; each is destroyed and freed exactly once.
    unsafe {
        dev.destroy_buffer(staging, None);
        dev.free_memory(smem, None);
    }
    Some(Rgb {
        image,
        memory,
        layout: vk::ImageLayout::GENERAL,
        family: vulkan::QueueFamily::Index(family),
    })
}

/// The `Repr::Rgb` generation a single-plane dmabuf import would have
/// produced — the wgpu wrap reports the image's real (GENERAL) state.
fn rgb_generation(
    shared: &SharedDevice,
    device: &vulkan::Device,
    rgb: Rgb,
    extent: (u32, u32),
    wait: Option<vulkan::Wait>,
    release: Option<vulkan::ReleaseSync>,
) -> Arc<vulkan::Generation> {
    // SAFETY: `rgb.image` was created on this device's VkDevice with the
    // format, extent and usage the descriptor declares; `TextureMemory::
    // External` records that the memory stays owned by the producer.
    let hal_tex = unsafe {
        shared
            .device
            .as_hal::<wgpu::hal::vulkan::Api>()
            .expect("vulkan device")
            .texture_from_raw(
                rgb.image,
                &wgpu::hal::TextureDescriptor {
                    label: Some("external frame (test)"),
                    size: wgpu::Extent3d {
                        width: extent.0,
                        height: extent.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::wgt::TextureUses::RESOURCE,
                    memory_flags: wgpu::hal::MemoryFlags::empty(),
                    view_formats: Vec::new(),
                },
                Some(Box::new(|| {})),
                wgpu::hal::vulkan::TextureMemory::External,
            )
    };
    // `RESOURCE` is wgpu-hal's GENERAL-layout state for color images —
    // the image's actual state at wrap time, never UNINITIALIZED.
    let pool = Some(dmabuf::create_pool(&device.shared, false).expect("pool"));
    // SAFETY: `hal_tex` was created by `texture_from_raw` on this device
    // above and the descriptor matches it field for field.
    let wrap = unsafe {
        shared
            .device
            .create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                hal_tex,
                &wgpu::TextureDescriptor {
                    label: Some("external frame (test)"),
                    size: wgpu::Extent3d {
                        width: extent.0,
                        height: extent.1,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
                wgpu::wgt::TextureUses::RESOURCE,
            )
    };
    Arc::new(vulkan::Generation {
        shared: device.shared.clone(),
        size: extent,
        color: FrameColor::BT709_VIDEO,
        alpha: crate::interop::RgbAlpha::Opaque,
        repr: vulkan::Repr::Rgb {
            format: wgpu::TextureFormat::Rgba8Unorm,
        },
        image: rgb.image,
        views: vulkan::sync::Views::Wrapped,
        conv: None,
        rgb_wrap: Some(wrap),
        bytes: 0,
        producer_layout: rgb.layout,
        producer_family: rgb.family,
        aspects: vk::ImageAspectFlags::COLOR,
        pool,
        sets: std::sync::Mutex::new(FxHashMap::default()),
        state: Arc::new(std::sync::Mutex::new(vulkan::State::Registered)),
        wait: std::sync::Mutex::new(wait),
        resolved_wait: std::sync::Mutex::new(None),
        release_sync: std::sync::Mutex::new(release),
        fence_semaphore: std::sync::Mutex::new(None),
        fence_fd: None,
        release_submitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        leases: AtomicUsize::new(0),
        parts: std::sync::Mutex::new(Some(vulkan::sync::Release {
            image: rgb.image,
            pool,
            views: vec![],
            conv: None,
            memory: vec![rgb.memory],
            semaphores: vec![],
            sync_payload: None,
            fence_semaphore: None,
            submitted_flag: None,
            state: None,
            acquired: false,
            producer_layout: rgb.layout,
            producer_family: rgb.family,
            aspects: vk::ImageAspectFlags::COLOR,
            lease: vulkan::sync::Lease::None,
            fence_fd: None,
            plane_fences: vec![],
        })),
        #[cfg(target_os = "android")]
        plane: None,
        plane_fences: std::sync::Mutex::new(vec![]),
    })
}

/// Presents `texture` into a fresh destination and returns its f32
/// pixels (the same helper pattern as `host_contracts`).
fn read_pixels(
    engine: &Engine<Gpu>,
    shared: &SharedDevice,
    source: &wgpu::Texture,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let (target, destinations) = TextureTarget::new((16, 16));
    let destination = engine.surface(target)?;
    let destination_texture = destinations.try_recv()?;
    let delivery =
        crate::interop::shader_delivery(shared.adapter.get_info().backend, &shared.device)?;
    let mut presenter = Presenter::new(&shared.device, delivery);
    presenter.texture(
        &shared.device,
        &shared.queue,
        &source.create_view(&wgpu::TextureViewDescriptor::default()),
        TextureOutput {
            texture: &destination_texture,
            color: OutputColor::LinearDisplayP3,
            alpha: OutputAlpha::Premultiplied,
            headroom: 1.0,
        },
    );
    Ok(destination.readback()?.pixels)
}

/// `engine.render` twice: the first submits acquisition, the second
/// proves a retained frame needs nothing again.
fn render_twice(engine: &Engine<Gpu>) {
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
}

#[test]
fn capability_record_and_honest_failures() {
    let Some((_, device)) = setup() else { return };
    let caps = *device.caps();
    eprintln!("vulkan external-frame caps: {caps:?}");
    // A dma-buf descriptor must not import where the driver lacks the
    // dma-buf capability — the contract returns Unsupported, never a copy.
    if !caps.external_memory_dma_buf {
        let desc = vulkan::DmaBuf {
            fourcc: 0,
            modifier: 0,
            size: (2, 2),
            planes: vec![],
            layout: vk::ImageLayout::GENERAL.as_raw().cast_unsigned(),
            memory: Vec::new(),
            producer_family: vulkan::QueueFamily::Index(0),
            sync: None,
            release: None,
            color: FrameColor::BT709_VIDEO,
            alpha: crate::interop::RgbAlpha::Opaque,
        };
        let result = device.import(vulkan::FrameSource::DmaBuf(Box::new(desc)));
        assert!(
            matches!(result, Err(NativeError::Unsupported(_))),
            "unsupported dmabuf import fails honestly: {result:?}"
        );
        eprintln!("labelled: dmabuf native import unavailable (no dma-buf ext)");
    }
    if !caps.external_semaphore_opaque_fd {
        eprintln!("labelled: external semaphore fd import/export unavailable");
    }
    if !caps.queue_family_foreign {
        eprintln!("labelled: foreign queue-family transfer unavailable");
    }
    if !caps.sampler_ycbcr_conversion {
        eprintln!("labelled: external-format combined-sampler path unavailable");
    }
    eprintln!("labelled: AHardwareBuffer native import not applicable (not Android)");
}

#[test]
fn nv12_plane_views_decode_in_place() {
    let Some((shared, device)) = setup() else {
        return;
    };
    let Some(nv12) = make_nv12(&shared, &device, (16, 16), 0x66, (0x80, 0x80)) else {
        eprintln!("lavapipe lacks G8_B8R8_2PLANE_420_UNORM — plane-view test labelled unavailable");
        return;
    };
    let generation = nv12_generation(&device, nv12, (16, 16), None, None);
    let frame = vulkan::Frame { generation };
    let external = ExternalFrame::native(frame).expect("native frame");
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
    sink.submit(external);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    render_twice(&engine);
    let pixels = read_pixels(&engine, &shared, &output).expect("readback");
    let pixel = pixels[8 * 16 + 8];
    // Neutral chroma decodes to grey on every channel.
    let max_dev = pixel[..3]
        .iter()
        .fold(0.0f32, |d, c| d.max((c - pixel[0]).abs()));
    assert!(
        pixel[0] > 0.02 && max_dev < 0.05 && pixel[3] > 0.99,
        "NV12 neutral chroma decodes to opaque grey: {pixel:?}"
    );
}

#[test]
fn same_device_rgb_wrap_decodes_in_place() {
    let Some((shared, device)) = setup() else {
        return;
    };
    let Some(rgb) = make_rgb(&shared, &device, (16, 16), [0xe0, 0x40, 0x20, 0xff]) else {
        eprintln!("unavailable: RGB image");
        return;
    };
    let generation = rgb_generation(&shared, &device, rgb, (16, 16), None, None);
    let frame = vulkan::Frame { generation };
    let external = ExternalFrame::native(frame).expect("native frame");
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
    sink.submit(external);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    render_twice(&engine);
    let pixels = read_pixels(&engine, &shared, &output).expect("readback");
    let pixel = pixels[8 * 16 + 8];
    assert!(
        pixel[0] > 0.5 && pixel[1] < 0.3 && pixel[2] < 0.3 && pixel[3] > 0.99,
        "same-device RGB wrap decoded red in place: {pixel:?}"
    );
}

#[test]
fn shared_acquisition_state_dedup_and_retention() {
    let Some((shared, device)) = setup() else {
        return;
    };
    let Some(rgb) = make_rgb(&shared, &device, (8, 8), [0x80, 0x20, 0x10, 0xff]) else {
        eprintln!("unavailable: RGB image");
        return;
    };
    let generation = rgb_generation(&shared, &device, rgb, (8, 8), None, None);
    // Two `ExternalFrame`s over the one generation — the shared
    // acquisition record deduplicates them.
    let external_a = ExternalFrame::native(vulkan::Frame {
        generation: generation.clone(),
    })
    .expect("native frame");
    let external_b = ExternalFrame::native(vulkan::Frame {
        generation: generation.clone(),
    })
    .expect("native frame");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared.clone()),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, _) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let (a, b) = (surface.layer(), surface.layer());
    let (pa, sa) = engine.frame_producer();
    sa.submit(external_a);
    let (pb, sb) = engine.frame_producer();
    sb.submit(external_b);
    surface.update(|tx| {
        tx[surface.root()].push(&a).push(&b);
        tx[&a].content(pa.at((8, 8)));
        tx[&b]
            .transform(cherenkov::kurbo::Affine::translate((4.0, 0.0)))
            .content(pb.at((8, 8)));
    });
    // Both attachments share one generation record: the first render
    // acquires once, and every later render is a plain retained draw.
    render_twice(&engine);
    // The submission-completion callback lands OwnedForRead; it fires on
    // a device poll, which a render alone does not guarantee.
    let _ = shared.device.poll(wgpu::PollType::Wait {
        submission_index: None,
        timeout: Some(std::time::Duration::from_secs(5)),
    });
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::OwnedForRead
    );
    assert_eq!(
        generation.leases.load(Ordering::Relaxed),
        2,
        "two slots lease the one generation"
    );
    // Detaching one layer keeps the generation live; detaching the last
    // retires it. The producer's slot holds the lease: it dies when the
    // producer's last clone does, which is the retirement.
    surface.update(|tx| {
        tx[surface.root()].remove(&a);
    });
    drop(a);
    drop(pa);
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    assert_eq!(
        generation.leases.load(Ordering::Relaxed),
        1,
        "the surviving attachment still holds the generation"
    );
    surface.update(|tx| {
        tx[surface.root()].remove(&b);
    });
    drop(b);
    drop(pb);
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    assert_eq!(generation.leases.load(Ordering::Relaxed), 0);
    let state = *generation.state.lock().expect("state");
    assert!(
        matches!(
            state,
            vulkan::State::Retiring | vulkan::State::ReleaseSubmitted | vulkan::State::Released
        ),
        "retirement queued on last unlease: {state:?}"
    );
}

#[test]
fn delayed_timeline_signal_stays_on_gpu() {
    let Some((shared, device)) = setup() else {
        return;
    };
    if !device.caps().timeline_semaphore {
        eprintln!("unavailable: timeline semaphores");
        return;
    }
    let (dev, _, _) = raw(&shared);
    // A host-owned timeline semaphore signalled late by a second thread.
    // SAFETY: `dev` is live and the create info describes a plain
    // timeline semaphore — nothing external is referenced.
    let semaphore = unsafe {
        let mut type_info =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
        dev.create_semaphore(
            &vk::SemaphoreCreateInfo::default().push_next(&mut type_info),
            None,
        )
    }
    .expect("timeline semaphore");
    let Some(rgb) = make_rgb(&shared, &device, (8, 8), [0x10, 0x20, 0x30, 0xff]) else {
        eprintln!("unavailable: RGB image");
        // SAFETY: `semaphore` was created above and no command references
        // it yet — destroying it here is its only destroy.
        unsafe { dev.destroy_semaphore(semaphore, None) };
        return;
    };
    let generation = rgb_generation(
        &shared,
        &device,
        rgb,
        (8, 8),
        Some(vulkan::Wait::Timeline {
            semaphore: semaphore.as_raw(),
            value: 1,
        }),
        Some(vulkan::ReleaseSync::Timeline {
            semaphore: semaphore.as_raw(),
            value: 2,
        }),
    );
    let external = ExternalFrame::native(vulkan::Frame { generation }).expect("native frame");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, _) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(external);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    // The wait point is genuinely unsignalled when this render runs:
    // nothing on the host signals it, so a CPU-waiting engine could not
    // return here at all — and any signal would raise the counter below.
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    // SAFETY: `semaphore` is a live timeline semaphore created above on
    // `dev` — a pure query.
    let counter = unsafe { dev.get_semaphore_counter_value(semaphore) }.expect("semaphore counter");
    assert_eq!(
        counter, 0,
        "engine returned with the wait point already signalled: {counter}"
    );
    // The producer signals the wait point on the host; the GPU-side wait
    // in the consuming submission now resolves.
    // SAFETY: `semaphore` is live and value 1 is a legal timeline signal
    // (it never regresses).
    unsafe {
        dev.signal_semaphore(
            &vk::SemaphoreSignalInfo::default()
                .semaphore(semaphore)
                .value(1),
        )
        .expect("signal");
    }
    // Retiring the frame queues the release submission; the next render
    // submits it behind the consuming submission on the one queue, so
    // the semaphore reaching the release point proves the timeline wait
    // executed on the GPU — without the wait, this point never arrives.
    // The frame lives on the producer: the last clone's drop retires it.
    drop(surface);
    drop(video);
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    // SAFETY: `semaphore` is live; the call only waits on it with a
    // timeout.
    let released = unsafe {
        dev.wait_semaphores(
            &vk::SemaphoreWaitInfo::default()
                .semaphores(&[semaphore])
                .values(&[2]),
            30_000_000_000,
        )
    };
    assert!(
        released.is_ok(),
        "release point never signalled: {released:?}"
    );
    // VUID-vkDestroySemaphore-semaphore-01149 forbids destroying the
    // semaphore while a queue command references it: the wait above
    // proves the release submission — the last reference — completed,
    // and `drop(engine)` joins the teardown queue regardless.
    drop(engine);
    // SAFETY: the wait above proved the release submission — the last
    // queue reference — completed, so VUID-vkDestroySemaphore-semaphore-
    // 01149 is met; `semaphore` is destroyed exactly once.
    unsafe { dev.destroy_semaphore(semaphore, None) };
}

/// A producer's retirement must release its frame's lease even when no
/// frame is rendered again: the retirement itself wakes the render loop
/// and submits the queued native release at once (#1691). The old loop
/// could only drain the retirement around a message, so an idle engine
/// left the release unsubmitted — this is a liveness check, and a few
/// seconds is generous for a submission the fix makes immediately.
#[test]
fn idle_retirement_submits_native_release() {
    let Some((shared, device)) = setup() else {
        return;
    };
    if !device.caps().timeline_semaphore {
        eprintln!("unavailable: timeline semaphores");
        return;
    }
    let (dev, _, _) = raw(&shared);
    // SAFETY: `dev` is live and the create info describes a plain
    // timeline semaphore — nothing external is referenced.
    let semaphore = unsafe {
        let mut type_info =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
        dev.create_semaphore(
            &vk::SemaphoreCreateInfo::default().push_next(&mut type_info),
            None,
        )
    }
    .expect("timeline semaphore");
    let Some(rgb) = make_rgb(&shared, &device, (8, 8), [0x10, 0x20, 0x30, 0xff]) else {
        eprintln!("unavailable: RGB image");
        // SAFETY: `semaphore` was created above and no command references
        // it yet — destroying it here is its only destroy.
        unsafe { dev.destroy_semaphore(semaphore, None) };
        return;
    };
    let generation = rgb_generation(
        &shared,
        &device,
        rgb,
        (8, 8),
        Some(vulkan::Wait::Timeline {
            semaphore: semaphore.as_raw(),
            value: 1,
        }),
        Some(vulkan::ReleaseSync::Timeline {
            semaphore: semaphore.as_raw(),
            value: 2,
        }),
    );
    let external = ExternalFrame::native(vulkan::Frame { generation }).expect("native frame");
    let engine = Engine::<Gpu>::new(GpuConfig {
        device: Some(shared),
        ..GpuConfig::default()
    })
    .expect("engine");
    let (target, _) = TextureTarget::new((16, 16));
    let surface = engine.surface(target).expect("surface");
    let layer = surface.layer();
    let (video, sink) = engine.frame_producer();
    sink.submit(external);
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(video.at((16, 16)));
    });
    assert!(matches!(engine.render(FrameTime::now()), Ok(Next::Idle)));
    // The producer signals the wait point on the host; the GPU-side
    // wait in the consuming submission resolves on the queue.
    // SAFETY: `semaphore` is live and value 1 is a legal timeline signal
    // (it never regresses).
    unsafe {
        dev.signal_semaphore(
            &vk::SemaphoreSignalInfo::default()
                .semaphore(semaphore)
                .value(1),
        )
        .expect("signal");
    }
    // Retiring the producer while the engine is idle posts its
    // retirement on the retirement queue — no render follows, so the
    // retirement alone must carry the release to submission.
    drop(surface);
    drop(video);
    // SAFETY: `semaphore` is live; the call only waits on it with a
    // timeout. The wait is a liveness check: a fixed loop submits the
    // release within a frame time, the old loop never submits it.
    let released = unsafe {
        dev.wait_semaphores(
            &vk::SemaphoreWaitInfo::default()
                .semaphores(&[semaphore])
                .values(&[2]),
            5_000_000_000,
        )
    };
    assert!(
        released.is_ok(),
        "idle retirement never submitted the release: {released:?}"
    );
    // The wait above proves the release submission — the last queue
    // reference — completed, and `drop(engine)` joins the teardown
    // queue regardless.
    drop(engine);
    // SAFETY: the wait above proved the release submission — the last
    // queue reference — completed, so VUID-vkDestroySemaphore-semaphore-
    // 01149 is met; `semaphore` is destroyed exactly once.
    unsafe { dev.destroy_semaphore(semaphore, None) };
}

#[test]
fn binary_state_machine_acquired_once() {
    let Some((shared, device)) = setup() else {
        return;
    };
    let Some(rgb) = make_rgb(&shared, &device, (8, 8), [0x30, 0x30, 0x30, 0xff]) else {
        eprintln!("unavailable: RGB image");
        return;
    };
    let (dev, _, family) = raw(&shared);
    let generation = rgb_generation(&shared, &device, rgb, (8, 8), None, None);
    // SAFETY: `dev` is live and `family` is the engine's queue family.
    let pool = unsafe {
        dev.create_command_pool(
            &vk::CommandPoolCreateInfo::default().queue_family_index(family),
            None,
        )
    }
    .expect("command pool");
    // SAFETY: `pool` was created above on `dev`.
    let cb = unsafe {
        dev.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .expect("command buffer")[0];
    // SAFETY: `cb` was allocated from `pool` and is unused.
    unsafe {
        dev.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())
            .expect("begin");
    }
    let mut native = vulkan::Native::new(device.shared).expect("native context");
    // SAFETY: `cb` is a recording command buffer on the shared device —
    // `stage_acquire`'s contract — and `generation`'s handles are live.
    let staged = unsafe { vulkan::stage_acquire(&generation, cb) }.expect("stage");
    assert!(staged.is_some());
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::AcquisitionPlanned
    );
    if let Some(pending) = staged {
        native.staged.push(pending);
    }
    // The consuming submission is accepted: the generation is submitted,
    // not yet owned — the queue's completion callback promotes it.
    vulkan::mark_submitted(&mut native);
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::AcquisitionSubmitted
    );
    assert!(native.staged.is_empty());
    vulkan::mark_owned(std::mem::take(&mut native.acquiring));
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::OwnedForRead
    );
    // The binary-state machine consumes the wait once: a later retained
    // draw on the same ordered queue stages nothing and does not wait.
    // SAFETY: `cb` is still recording on the shared device.
    let restaged = unsafe { vulkan::stage_acquire(&generation, cb) }.expect("restage");
    assert!(restaged.is_none(), "acquired generation never re-waits");
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::OwnedForRead
    );
    vulkan::cancel_staged(&mut native);
    // SAFETY: `cb` was never submitted and `pool` is destroyed once.
    unsafe { dev.destroy_command_pool(pool, None) };
}

#[test]
fn opaque_fd_wait_is_honestly_unsupported_on_lavapipe() {
    let Some((shared, device)) = setup() else {
        return;
    };
    if device.caps().external_semaphore_opaque_fd {
        return;
    }
    let Some(rgb) = make_rgb(&shared, &device, (8, 8), [0x40, 0x40, 0x40, 0xff]) else {
        eprintln!("unavailable: RGB image");
        return;
    };
    // `/dev/null` stands in for a payload fd: the descriptor is well
    // formed, only the capability is absent — the error must be
    // `Unsupported`, not a driver failure.
    let fd = std::os::fd::OwnedFd::from(std::fs::File::open("/dev/null").expect("dev null"));
    let generation = rgb_generation(
        &shared,
        &device,
        rgb,
        (8, 8),
        Some(vulkan::Wait::OpaqueFd { fd }),
        None,
    );
    let (dev, _, family) = raw(&shared);
    // SAFETY: `dev` is live and `family` is the engine's queue family.
    let pool = unsafe {
        dev.create_command_pool(
            &vk::CommandPoolCreateInfo::default().queue_family_index(family),
            None,
        )
    }
    .expect("command pool");
    // SAFETY: `pool` was created above on `dev`.
    let cb = unsafe {
        dev.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .expect("command buffer")[0];
    // SAFETY: `cb` was allocated from `pool` and is unused.
    unsafe {
        dev.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())
            .expect("begin");
    }
    // SAFETY: `cb` is recording on the shared device; a failed resolve
    // leaves the generation unacquired rather than corrupting it.
    let result = unsafe { vulkan::stage_acquire(&generation, cb) };
    // SAFETY: `cb` was never submitted and `pool` is destroyed once.
    unsafe { dev.destroy_command_pool(pool, None) };
    assert!(
        matches!(result, Err(vulkan::NativeError::Unsupported(_))),
        "fd payload import without the capability is Unsupported"
    );
    // A failed resolve leaves the frame unacquired.
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::Registered
    );
}

#[test]
fn plan_cancellation_restores_the_frame() {
    let Some((shared, device)) = setup() else {
        return;
    };
    let Some(rgb) = make_rgb(&shared, &device, (8, 8), [0x20, 0x20, 0x20, 0xff]) else {
        eprintln!("unavailable: RGB image");
        return;
    };
    // A timeline wait survives cancellation: the resolved payload returns
    // to the generation, and a later plan stages it again.
    let (dev, _, family) = raw(&shared);
    // SAFETY: `dev` is live and the create info describes a plain
    // timeline semaphore — nothing external is referenced.
    let semaphore = unsafe {
        let mut type_info =
            vk::SemaphoreTypeCreateInfo::default().semaphore_type(vk::SemaphoreType::TIMELINE);
        dev.create_semaphore(
            &vk::SemaphoreCreateInfo::default().push_next(&mut type_info),
            None,
        )
    }
    .expect("timeline semaphore");
    let generation = rgb_generation(
        &shared,
        &device,
        rgb,
        (8, 8),
        Some(vulkan::Wait::Timeline {
            semaphore: semaphore.as_raw(),
            value: 1,
        }),
        None,
    );
    // Stage an acquire into a scratch context through the engine's own
    // path: `stage_acquire` is the unit a cancelled encode unwinds. The
    // barrier is recorded into a real buffer the submission never sees.
    // SAFETY: `dev` is live and `family` is the engine's queue family.
    let pool = unsafe {
        dev.create_command_pool(
            &vk::CommandPoolCreateInfo::default().queue_family_index(family),
            None,
        )
    }
    .expect("command pool");
    // SAFETY: `pool` was created above on `dev`.
    let cb = unsafe {
        dev.allocate_command_buffers(
            &vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1),
        )
    }
    .expect("command buffer")[0];
    // SAFETY: `cb` was allocated from `pool` and is unused.
    unsafe {
        dev.begin_command_buffer(cb, &vk::CommandBufferBeginInfo::default())
            .expect("begin");
    }
    let mut native = vulkan::Native::new(device.shared).expect("native context");
    // SAFETY: `cb` is a recording command buffer on the shared device —
    // `stage_acquire`'s contract — and `generation`'s handles are live.
    let staged = unsafe { vulkan::stage_acquire(&generation, cb) }.expect("stage");
    assert!(staged.is_some(), "unconsumed generation stages an acquire");
    if let Some(pending) = staged {
        native.staged.push(pending);
    }
    vulkan::cancel_staged(&mut native);
    // SAFETY: `cb` was never submitted and `pool` is destroyed once.
    unsafe { dev.destroy_command_pool(pool, None) };
    assert_eq!(
        *generation.state.lock().expect("state"),
        vulkan::State::Registered,
        "a cancelled plan leaves the frame unacquired"
    );
    // The resolved wait went back to the generation — same semaphore,
    // same point — and the next plan re-stages it instead of importing
    // a fresh one.
    let guard = generation.resolved_wait.lock().expect("resolved wait");
    let (restored_semaphore, restored_value, restored_owned) = {
        let p = guard.as_ref().expect("restored payload");
        (p.semaphore, p.value, p.owned)
    };
    drop(guard);
    assert_eq!(restored_semaphore, semaphore);
    assert_eq!(restored_value, Some(1));
    assert!(!restored_owned);
    // SAFETY: the semaphore was never submitted to a queue — no command
    // references it, so destroying it is safe and happens exactly once.
    unsafe { dev.destroy_semaphore(semaphore, None) };
}
