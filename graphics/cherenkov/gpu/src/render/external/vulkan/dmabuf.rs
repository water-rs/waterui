//! Linux dma-buf import.
//!
//! The descriptor is complete: the DRM fourcc and modifier, the extent, an
//! owned fd per memory plane, the offset and stride of every colour-format
//! plane, and the producer's layout/ownership state and synchronization.
//! Nothing is inferred from the extent, and DRM memory planes are never
//! equated with Vulkan colour planes — `planes` carries one entry per
//! image aspect while `memory` carries one fd per memory plane.
//!
//! Known-format multiplanar buffers (NV12/P010) import as one multiplanar
//! `VkImage` bound through native integer plane views; ordinary
//! single-plane RGB buffers wrap as a `wgpu::Texture` through
//! `texture_from_raw` so the ordinary external path draws them.

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

use ash::vk;
use rustc_hash::FxHashMap;

use super::{Frame, NativeError, Shared, sync};

// DRM fourcc codes (`drm_fourcc.h`), kept as named constants so the import
// table reads `NV12`, not a magic dword. The single-plane RGBA fourccs are
// `pub`: `interop::dmabuf` re-exports them for hosts declaring the formats
// they can import (#1687).
/// `DRM_FORMAT_ARGB8888` — 8-bit BGRA, alpha filled 1 on `X` variants.
pub const DRM_FORMAT_ARGB8888: u32 = 0x3432_5241;
/// `DRM_FORMAT_XRGB8888` — 8-bit BGRA without alpha.
pub const DRM_FORMAT_XRGB8888: u32 = 0x3432_5258;
/// `DRM_FORMAT_ABGR8888` — 8-bit RGBA, alpha filled 1 on `X` variants.
pub const DRM_FORMAT_ABGR8888: u32 = 0x3432_4241;
/// `DRM_FORMAT_XBGR8888` — 8-bit RGBA without alpha.
pub const DRM_FORMAT_XBGR8888: u32 = 0x3432_4258;
const DRM_FORMAT_NV12: u32 = 0x3231_564E;
const DRM_FORMAT_P010: u32 = 0x3031_3050;

/// `DRM_FORMAT_MOD_INVALID` — a descriptor must name a real modifier.
const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;

/// What a fourcc describes: the Vulkan image format, the colour-plane
/// count, and — for multiplanar — the integer plane-view formats.
#[derive(Clone, Copy)]
enum Shape {
    /// Ordinary single-plane RGB(A) — wraps through `texture_from_raw`.
    Rgb {
        format: vk::Format,
        wgpu: wgpu::TextureFormat,
    },
    /// Two-plane 4:2:0 — the image format, the per-plane integer view
    /// formats and the shader kind discriminant.
    Planes {
        format: vk::Format,
        y: vk::Format,
        uv: vk::Format,
        kind: u32,
    },
}

const fn shape(fourcc: u32) -> Option<Shape> {
    Some(match fourcc {
        DRM_FORMAT_ARGB8888 | DRM_FORMAT_XRGB8888 => Shape::Rgb {
            format: vk::Format::B8G8R8A8_UNORM,
            wgpu: wgpu::TextureFormat::Bgra8Unorm,
        },
        DRM_FORMAT_ABGR8888 | DRM_FORMAT_XBGR8888 => Shape::Rgb {
            format: vk::Format::R8G8B8A8_UNORM,
            wgpu: wgpu::TextureFormat::Rgba8Unorm,
        },
        DRM_FORMAT_NV12 => Shape::Planes {
            format: vk::Format::G8_B8R8_2PLANE_420_UNORM,
            y: vk::Format::R8_UINT,
            uv: vk::Format::R8G8_UINT,
            kind: super::super::KIND_NV12,
        },
        DRM_FORMAT_P010 => Shape::Planes {
            format: vk::Format::G10X6_B10X6R10X6_2PLANE_420_UNORM_3PACK16,
            y: vk::Format::R16_UINT,
            uv: vk::Format::R16G16_UINT,
            kind: super::super::KIND_P010,
        },
        _ => return None,
    })
}

/// The colour plane's aspect mask by plane index.
const fn plane_aspect(plane: u32) -> vk::ImageAspectFlags {
    match plane {
        0 => vk::ImageAspectFlags::PLANE_0,
        1 => vk::ImageAspectFlags::PLANE_1,
        _ => vk::ImageAspectFlags::PLANE_2,
    }
}

/// Queries the physical device for the exact format/modifier/usage/handle
/// combination the import needs before any object is created.
fn support_checked(
    shared: &Shared,
    format: vk::Format,
    modifier: u64,
    flags: vk::ImageCreateFlags,
) -> Result<(), NativeError> {
    let mut modifier_info = vk::PhysicalDeviceImageDrmFormatModifierInfoEXT::default()
        .drm_format_modifier(modifier)
        .sharing_mode(vk::SharingMode::EXCLUSIVE);
    let mut external_info = vk::PhysicalDeviceExternalImageFormatInfo::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let info = vk::PhysicalDeviceImageFormatInfo2::default()
        .format(format)
        .ty(vk::ImageType::TYPE_2D)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .flags(flags)
        .push_next(&mut external_info)
        .push_next(&mut modifier_info);
    let mut props = vk::ImageFormatProperties2::default();
    // SAFETY: `shared`'s instance/physical_device are live and `info`/
    // `props` are correctly-chained parameter and out structs.
    unsafe {
        shared
            .instance
            .get_physical_device_image_format_properties2(shared.physical_device, &info, &mut props)
    }
    .map_err(|_| NativeError::Unsupported("format/modifier/usage rejected by driver"))?;
    Ok(())
}

/// The descriptor pool a native-bound generation allocates its set-1
/// descriptors from.
///
/// For external-format conversions the combined sampler carries the
/// conversion's descriptor requirements — the pool sizes for
/// `COMBINED_IMAGE_SAMPLER` accordingly.
///
/// # Errors
/// [`NativeError`] when pool creation fails.
pub fn create_pool(shared: &Shared, combined: bool) -> Result<vk::DescriptorPool, NativeError> {
    let mut sizes = vec![
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::SAMPLED_IMAGE,
            descriptor_count: 64,
        },
        vk::DescriptorPoolSize {
            ty: vk::DescriptorType::UNIFORM_BUFFER,
            descriptor_count: 16,
        },
    ];
    if combined {
        sizes.push(vk::DescriptorPoolSize {
            ty: vk::DescriptorType::COMBINED_IMAGE_SAMPLER,
            descriptor_count: 64,
        });
    }
    // SAFETY: `shared.vk.device` is live and the pool sizes are
    // compile-time constants built above.
    unsafe {
        shared.vk.device.create_descriptor_pool(
            &vk::DescriptorPoolCreateInfo::default()
                // Individual frees feed the bounded set-1 cache's
                // eviction path; the pool destroy at release frees the
                // rest.
                .flags(vk::DescriptorPoolCreateFlags::FREE_DESCRIPTOR_SET)
                .max_sets(16)
                .pool_sizes(&sizes),
            None,
        )
    }
    .map_err(NativeError::from)
}

/// The bound memory handles, kept for the release.
struct Bound {
    memory: Vec<vk::DeviceMemory>,
    bytes: u64,
}

/// Imports `desc` into a new frame generation.
///
/// On failure every fd still owned by Cherenkov is closed; once a
/// `vkAllocateMemory` import succeeds the corresponding `OwnedFd` is
/// cleared so nothing closes it twice.
#[expect(
    clippy::too_many_lines,
    reason = "import, validate, allocate, bind and view creation run in one"
)]
pub fn import(shared: &Arc<Shared>, mut desc: super::DmaBuf) -> Result<Frame, NativeError> {
    if !shared.caps.external_memory_dma_buf || !shared.caps.external_memory_fd {
        return Err(NativeError::Unsupported(
            "dma-buf external memory is not enabled on this device",
        ));
    }
    if !shared.caps.image_drm_format_modifier {
        return Err(NativeError::Unsupported(
            "VK_EXT_image_drm_format_modifier is not enabled",
        ));
    }
    let Some(shape) = shape(desc.fourcc) else {
        return Err(NativeError::Unsupported("unmapped DRM fourcc"));
    };
    if desc.size.0 == 0 || desc.size.1 == 0 {
        return Err(NativeError::Invalid("empty extent"));
    }
    if desc.modifier == DRM_FORMAT_MOD_INVALID {
        return Err(NativeError::Invalid("DRM_FORMAT_MOD_INVALID modifier"));
    }
    if desc.memory.is_empty() {
        return Err(NativeError::Invalid("no memory planes"));
    }
    let colour_planes = match shape {
        Shape::Rgb { .. } => 1,
        Shape::Planes { .. } => 2,
    };
    if desc.planes.len() != colour_planes {
        return Err(NativeError::Invalid(
            "colour-plane count does not match the fourcc",
        ));
    }
    for plane in &desc.planes {
        if plane.memory as usize >= desc.memory.len() {
            return Err(NativeError::Invalid("plane references no memory"));
        }
        if plane.stride == 0 {
            return Err(NativeError::Invalid("zero stride"));
        }
    }
    let disjoint = desc.memory.len() > 1;
    if disjoint && desc.memory.len() != desc.planes.len() {
        return Err(NativeError::Invalid(
            "disjoint import needs one fd per colour plane",
        ));
    }
    // XRGB/XBGR fourccs have no alpha bits — the plane's alpha contract is
    // always opaque.
    let opaque_only = matches!(
        desc.fourcc,
        DRM_FORMAT_XRGB8888 | DRM_FORMAT_XBGR8888 | DRM_FORMAT_NV12 | DRM_FORMAT_P010
    );
    if opaque_only && desc.alpha != crate::interop::RgbAlpha::Opaque {
        return Err(NativeError::Invalid("the format carries no alpha"));
    }
    let layout = vk::ImageLayout::from_raw(desc.layout.cast_signed());
    if layout == vk::ImageLayout::UNDEFINED {
        // A producer that wrote pixels must name its real layout; UNDEFINED
        // discards them by contract.
        return Err(NativeError::Invalid(
            "producer layout UNDEFINED discards written pixels",
        ));
    }
    let mut flags = vk::ImageCreateFlags::MUTABLE_FORMAT;
    match shape {
        Shape::Planes { .. } => {
            flags |= vk::ImageCreateFlags::EXTENDED_USAGE;
            if disjoint {
                flags |= vk::ImageCreateFlags::DISJOINT;
            }
        }
        Shape::Rgb { .. } => {}
    }
    support_checked(shared, format_of(shape), desc.modifier, flags)?;

    let dev = &shared.vk.device;
    let layout_infos: Vec<vk::SubresourceLayout> = desc
        .planes
        .iter()
        .map(|p| {
            vk::SubresourceLayout::default()
                .offset(u64::from(p.offset))
                .row_pitch(u64::from(p.stride))
        })
        .collect();
    let plane_formats: Vec<vk::Format> = match shape {
        Shape::Rgb { .. } => Vec::new(),
        Shape::Planes { y, uv, .. } => vec![y, uv],
    };
    let mut modifier_info = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(desc.modifier)
        .plane_layouts(&layout_infos);
    let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT);
    let mut format_list = vk::ImageFormatListCreateInfo::default().view_formats(&plane_formats);
    let mut create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format_of(shape))
        .extent(vk::Extent3D {
            width: desc.size.0,
            height: desc.size.1,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .flags(flags)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut modifier_info)
        .push_next(&mut external_info);
    if !plane_formats.is_empty() {
        create_info = create_info.push_next(&mut format_list);
    }
    // SAFETY: `dev` is live and `create_info` only references the
    // modifier/format-list structs built above from the descriptor.
    let image = unsafe { dev.create_image(&create_info, None) }.map_err(NativeError::from)?;

    // From here every failure path must close the still-owned fds; the
    // descriptor moved its `memory` Vec into `fds`.
    let mut fds: Vec<Option<OwnedFd>> = std::mem::take(&mut desc.memory)
        .into_iter()
        .map(Some)
        .collect();
    let result = bind_memory(shared, image, &desc, disjoint, &mut fds)
        .and_then(|bound| finish(shared, image, bound, desc, shape, layout));
    match result {
        Ok(frame) => Ok(frame),
        Err(err) => {
            // SAFETY: `image` was created above on `dev` and is destroyed
            // exactly once on this error path.
            unsafe { dev.destroy_image(image, None) };
            // The remaining `Some` fds drop — close — here; consumed ones
            // were cleared at import so nothing closes twice.
            drop(fds);
            Err(err)
        }
    }
}

const fn format_of(shape: Shape) -> vk::Format {
    match shape {
        Shape::Rgb { format, .. } | Shape::Planes { format, .. } => format,
    }
}

/// Imports each memory plane's fd and binds it per the disjoint contract.
/// On success the imported `OwnedFd` entries are cleared (`None`); on
/// failure every allocation is rolled back and still-owned fds stay set.
/// Allocate `reqs` importing `fd`; dedicated-allocation requirements
/// chain in automatically.
unsafe fn import_memory(
    dev: &ash::Device,
    image: vk::Image,
    reqs: &vk::MemoryRequirements,
    dedicated: &vk::MemoryDedicatedRequirements,
    fd: std::os::fd::RawFd,
) -> Result<vk::DeviceMemory, NativeError> {
    let mut import = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT)
        .fd(fd);
    let mut dedicated_alloc = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(reqs.size)
        .push_next(&mut import);
    if dedicated.prefers_dedicated_allocation == vk::TRUE
        || dedicated.requires_dedicated_allocation == vk::TRUE
    {
        alloc = alloc.push_next(&mut dedicated_alloc);
    }
    // The driver's memory-type bitmask is producer-dictated; the first
    // bit is the contract the dma-buf import API exposes.
    alloc = alloc.memory_type_index(reqs.memory_type_bits.trailing_zeros());
    // SAFETY: `dev` is live; `alloc` references `reqs` (just queried for
    // `image`), the live dma-buf `fd` to import, and the dedicated chain.
    unsafe { dev.allocate_memory(&alloc, None) }.map_err(NativeError::from)
}

fn bind_memory(
    shared: &Shared,
    image: vk::Image,
    desc: &super::DmaBuf,
    disjoint: bool,
    fds: &mut [Option<OwnedFd>],
) -> Result<Bound, NativeError> {
    let dev = &shared.vk.device;
    let mut memory = Vec::with_capacity(fds.len());
    let mut bytes = 0u64;

    let result = (|| -> Result<(), NativeError> {
        if disjoint {
            for (plane_index, plane) in desc.planes.iter().enumerate() {
                let mut plane_info = vk::ImagePlaneMemoryRequirementsInfo::default().plane_aspect(
                    plane_aspect(u32::try_from(plane_index).expect("plane index")),
                );
                let info = vk::ImageMemoryRequirementsInfo2::default()
                    .image(image)
                    .push_next(&mut plane_info);
                let mut dedicated = vk::MemoryDedicatedRequirements::default();
                let mut reqs2 = vk::MemoryRequirements2::default().push_next(&mut dedicated);
                // SAFETY: `info` references the live `image` and `reqs2`
                // is a live out struct chained to `dedicated`.
                unsafe { dev.get_image_memory_requirements2(&info, &mut reqs2) };
                let reqs = reqs2.memory_requirements;
                let mi = plane.memory as usize;
                let fd = fds[mi]
                    .as_ref()
                    .ok_or(NativeError::Invalid("fd reused"))?
                    .as_raw_fd();
                // SAFETY: `fd` is an owned dma-buf fd from `desc`'s
                // descriptor, imported exactly once into `mem`.
                let mem = unsafe { import_memory(dev, image, &reqs, &dedicated, fd) }?;
                fds[mi] = None;
                let mut plane_info = vk::BindImagePlaneMemoryInfo::default().plane_aspect(
                    plane_aspect(u32::try_from(plane_index).expect("plane index")),
                );
                let bind = vk::BindImageMemoryInfo::default()
                    .image(image)
                    .memory(mem)
                    .push_next(&mut plane_info);
                // SAFETY: `bind` references the live `image`, the just-
                // imported `mem`, and this plane's aspect — the disjoint
                // contract binds one allocation per plane.
                match unsafe { dev.bind_image_memory2(&[bind]) } {
                    Ok(()) => {
                        memory.push(mem);
                        bytes += reqs.size;
                    }
                    Err(err) => {
                        // SAFETY: `mem` was imported above, is not bound
                        // to the image, and is freed exactly once.
                        unsafe { dev.free_memory(mem, None) };
                        return Err(err.into());
                    }
                }
            }
        } else {
            let mut dedicated = vk::MemoryDedicatedRequirements::default();
            let info = vk::ImageMemoryRequirementsInfo2::default().image(image);
            let mut reqs2 = vk::MemoryRequirements2::default().push_next(&mut dedicated);
            // SAFETY: `info` references the live `image` and `reqs2` is a
            // live out struct chained to `dedicated`.
            unsafe { dev.get_image_memory_requirements2(&info, &mut reqs2) };
            let reqs = reqs2.memory_requirements;
            let mi = desc.planes[0].memory as usize;
            let fd = fds[mi]
                .as_ref()
                .ok_or(NativeError::Invalid("fd reused"))?
                .as_raw_fd();
            // SAFETY: `fd` is an owned dma-buf fd from `desc`'s
            // descriptor, imported exactly once into `mem`.
            let mem = unsafe { import_memory(dev, image, &reqs, &dedicated, fd) }?;
            fds[mi] = None;
            // SAFETY: `image` is live and `mem` was just imported for
            // it; offset 0 binds the whole requirement.
            match unsafe { dev.bind_image_memory(image, mem, 0) } {
                Ok(()) => {
                    memory.push(mem);
                    bytes += reqs.size;
                }
                Err(err) => {
                    // SAFETY: `mem` was imported above, is not bound to
                    // the image, and is freed exactly once.
                    unsafe { dev.free_memory(mem, None) };
                    return Err(err.into());
                }
            }
        }
        Ok(())
    })();

    if let Err(err) = result {
        for mem in memory {
            // SAFETY: each `mem` was allocated by `import_memory` and is
            // freed exactly once on this rollback path.
            unsafe { dev.free_memory(mem, None) };
        }
        return Err(err);
    }
    Ok(Bound { memory, bytes })
}

/// Finishes the import: plane views or the wgpu wrap, the descriptor pool
/// and the generation record. `desc`'s synchronization, colour and release
/// fields move into the generation.
#[expect(
    clippy::too_many_lines,
    reason = "per-plane view creation plus generation assembly is one unit"
)]
fn finish(
    shared: &Arc<Shared>,
    image: vk::Image,
    bound: Bound,
    desc: super::DmaBuf,
    shape: Shape,
    layout: vk::ImageLayout,
) -> Result<Frame, NativeError> {
    let dev = &shared.vk.device;
    let (repr, views, aspects, rgb_wrap, pool, view_handles) = match shape {
        Shape::Planes { y, uv, kind, .. } => {
            let mut handles = Vec::with_capacity(2);
            for (plane, fmt) in [y, uv].iter().enumerate() {
                // SAFETY: `image` is live on `dev` and `fmt`/aspect are
                // the descriptor's per-plane pair.
                match unsafe {
                    dev.create_image_view(
                        &vk::ImageViewCreateInfo::default()
                            .image(image)
                            .view_type(vk::ImageViewType::TYPE_2D)
                            .format(*fmt)
                            .subresource_range(vk::ImageSubresourceRange {
                                aspect_mask: plane_aspect(
                                    u32::try_from(plane).expect("plane index"),
                                ),
                                base_mip_level: 0,
                                level_count: 1,
                                base_array_layer: 0,
                                layer_count: 1,
                            }),
                        None,
                    )
                } {
                    Ok(view) => handles.push(view),
                    Err(err) => {
                        for v in handles {
                            // SAFETY: each `v` was created above on `dev`
                            // and is destroyed exactly once.
                            unsafe { dev.destroy_image_view(v, None) };
                        }
                        return Err(err.into());
                    }
                }
            }
            (
                super::Repr::Planes { kind },
                sync::Views::Planes {
                    y: handles[0],
                    uv: handles[1],
                },
                vk::ImageAspectFlags::COLOR,
                None,
                Some(create_pool(shared, false)?),
                handles,
            )
        }
        Shape::Rgb { wgpu, .. } => {
            // Ordinary single-plane RGB wraps as a wgpu texture tracking
            // the producer's actual state — never a convenience
            // UNINITIALIZED: the acquire barrier lands the image in
            // SHADER_READ_ONLY_OPTIMAL before first use, which is what
            // `RESOURCE` declares.
            // SAFETY: `image` was created on `shared`'s VkDevice with the
            // format, extent and usage the descriptor declares; the
            // comment above records why RESOURCE is the honest state.
            let texture = unsafe {
                let hal_device = shared
                    .wgpu
                    .as_hal::<wgpu::hal::vulkan::Api>()
                    .expect("vulkan device");
                let hal_tex = hal_device.texture_from_raw(
                    image,
                    &wgpu::hal::TextureDescriptor {
                        label: Some("external frame"),
                        size: wgpu::Extent3d {
                            width: desc.size.0,
                            height: desc.size.1,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu,
                        usage: wgpu::wgt::TextureUses::RESOURCE,
                        memory_flags: wgpu::hal::MemoryFlags::empty(),
                        view_formats: Vec::new(),
                    },
                    // The generation's `Release` destroys the image and
                    // memory; the drop guard only releases wgpu's handle.
                    Some(Box::new(|| {})),
                    wgpu::hal::vulkan::TextureMemory::External,
                );
                shared
                    .wgpu
                    .create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                        hal_tex,
                        &wgpu::TextureDescriptor {
                            label: Some("external frame"),
                            size: wgpu::Extent3d {
                                width: desc.size.0,
                                height: desc.size.1,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu,
                            usage: wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        },
                        wgpu::wgt::TextureUses::RESOURCE,
                    )
            };
            (
                super::Repr::Rgb { format: wgpu },
                sync::Views::Wrapped,
                vk::ImageAspectFlags::COLOR,
                Some(texture),
                None,
                Vec::new(),
            )
        }
    };

    // A `FenceFd` release needs the export semaphore at import time.
    let fence_semaphore = match &desc.release {
        Some(super::ReleaseSync::FenceFd) => {
            if !shared.caps.external_semaphore_sync_fd {
                return Err(NativeError::Unsupported(
                    "SYNC_FD semaphore export is not enabled",
                ));
            }
            let mut export = vk::ExportSemaphoreCreateInfo::default()
                .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            let info = vk::SemaphoreCreateInfo::default().push_next(&mut export);
            // SAFETY: `dev` is live and `info` chains the export info
            // for a SYNC_FD semaphore — the capability was checked above.
            Some(unsafe { dev.create_semaphore(&info, None) }.map_err(NativeError::from)?)
        }
        _ => None,
    };

    let lease = sync::Lease::None;

    Ok(Frame {
        generation: Arc::new(sync::Generation {
            shared: Arc::clone(shared),
            size: desc.size,
            color: desc.color,
            alpha: desc.alpha,
            repr,
            image,
            views,
            conv: None,
            rgb_wrap,
            bytes: bound.bytes,
            producer_layout: layout,
            producer_family: desc.producer_family,
            aspects,
            pool,
            sets: std::sync::Mutex::new(FxHashMap::default()),
            state: Arc::new(std::sync::Mutex::new(sync::State::Registered)),
            wait: std::sync::Mutex::new(desc.sync),
            resolved_wait: std::sync::Mutex::new(None),
            release_sync: std::sync::Mutex::new(desc.release),
            fence_semaphore: std::sync::Mutex::new(fence_semaphore),
            fence_fd: fence_semaphore
                .is_some()
                .then(|| Arc::new(std::sync::Mutex::new(sync::FenceFd::Pending))),
            release_submitted: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            leases: std::sync::atomic::AtomicUsize::new(0),
            parts: std::sync::Mutex::new(Some(sync::Release {
                image,
                pool,
                views: view_handles,
                conv: None,
                memory: bound.memory,
                semaphores: Vec::new(),
                sync_payload: None,
                fence_semaphore: None,
                fence_fd: None,
                submitted_flag: None,
                state: None,
                acquired: false,
                producer_layout: layout,
                producer_family: desc.producer_family,
                aspects,
                lease,
                plane_fences: Vec::new(),
            })),
            #[cfg(target_os = "android")]
            plane: None,
            plane_fences: std::sync::Mutex::new(Vec::new()),
        }),
    })
}
