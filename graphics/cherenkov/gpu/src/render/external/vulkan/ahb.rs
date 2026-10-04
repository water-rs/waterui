//! Android `AHardwareBuffer` import.
//!
//! The buffer is retained for the frame's lease, described through
//! `AHardwareBuffer_describe`, and its `VkAndroidHardwareBufferProperties`
//! give the allocation size, memory type bits and — for opaque YCbCr
//! buffers — the `externalFormat` identifier and the driver's suggested
//! conversion parameters. The `suggestedYcbcr*` fields are advisory hints
//! with no matching `VUID` — the conversion may use the frame's declared
//! `FrameColor` verbatim; only `samplerYcbcrConversionComponents` and the
//! `COSITED_CHROMA_SAMPLES` format feature are hard requirements.
//!
//! The producer contract is fixed: `VK_QUEUE_FAMILY_FOREIGN_EXT` and no
//! prior Vulkan layout — the acquire barrier is `UNDEFINED` →
//! `SHADER_READ_ONLY_OPTIMAL` with a `FOREIGN_EXT` → engine transfer.

#![cfg(target_os = "android")]

use std::sync::Arc;

use ash::vk;
use rustc_hash::FxHashMap;

use super::{
    Ahb, Frame, NativeError, QueueFamily, Shared, chroma_location, dmabuf::create_pool,
    required_model, required_range, sync, ycbcr,
};

/// Imports `desc`, consuming it, as one frame generation.
pub fn import(shared: &Arc<Shared>, desc: Ahb) -> Result<Frame, NativeError> {
    if shared.vk.ahb.is_none() {
        return Err(NativeError::Unsupported(
            "VK_ANDROID_external_memory_android_hardware_buffer is not enabled",
        ));
    }
    let buffer = desc.buffer.cast::<ndk_sys::AHardwareBuffer>();
    if buffer.is_null() {
        return Err(NativeError::Invalid("null AHardwareBuffer"));
    }
    // The producer lease: retained until the frame retires.
    unsafe { ndk_sys::AHardwareBuffer_acquire(buffer) };
    let result = import_inner(shared, desc, buffer);
    if result.is_err() {
        unsafe { ndk_sys::AHardwareBuffer_release(buffer) };
    }
    result
}

fn import_inner(
    shared: &Arc<Shared>,
    desc: Ahb,
    buffer: *mut ndk_sys::AHardwareBuffer,
) -> Result<Frame, NativeError> {
    let mut ahb_desc = ndk_sys::AHardwareBuffer_Desc {
        width: 0,
        height: 0,
        layers: 0,
        format: 0,
        usage: 0,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    unsafe { ndk_sys::AHardwareBuffer_describe(buffer, &raw mut ahb_desc) };
    if ahb_desc.width == 0 || ahb_desc.height == 0 || ahb_desc.layers != 1 {
        return Err(NativeError::Invalid("buffer must be a single-layer frame"));
    }

    // Query the Vulkan properties: allocation size, memory type bits and —
    // for opaque buffers — the external format and the driver's suggested
    // conversion contract.
    let mut format_props = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
    let mut props =
        vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format_props);
    let ahb_loader = shared.vk.ahb.as_ref().expect("checked at import");
    unsafe { ahb_loader.get_android_hardware_buffer_properties(buffer as *const _, &mut props) }
        .map_err(NativeError::from)?;
    // The chained query borrows `format_props` through `props`; lift every
    // reported field into a local so the image-create chain can borrow
    // `format_props`-derived values freely below.
    let allocation_size = props.allocation_size;
    let memory_type_bits = props.memory_type_bits;
    let vk_format = format_props.format;
    let external_id = format_props.external_format;
    let format_features = format_props.format_features;
    let required_mapping = format_props.sampler_ycbcr_conversion_components;
    if memory_type_bits == 0 {
        return Err(NativeError::Invalid("no compatible memory type"));
    }

    let external = vk_format == vk::Format::UNDEFINED;
    if external && !shared.caps.sampler_ycbcr_conversion {
        return Err(NativeError::Unsupported(
            "external-format buffers need sampler_ycbcr_conversion",
        ));
    }
    if external && external_id == 0 {
        return Err(NativeError::Invalid("external buffer reports no format"));
    }

    // The frame's declared colour contract is the conversion contract —
    // model, range and chroma siting are applied verbatim. The driver's
    // `suggestedYcbcr*` values are advisory only (they are what the driver
    // suggests, not what it requires; a camera or video NV12 buffer often
    // carries a generic suggestion the frame's declared contract
    // deliberately differs from). The one capability gate is the format's
    // own feature set: cosited siting needs `COSITED_CHROMA_SAMPLES`, and
    // `samplerYcbcrConversionComponents` is checked verbatim when the
    // conversion is created.
    if external {
        // Cosited siting needs the format feature.
        if (desc.color.chroma_siting.x == crate::interop::ChromaOffset::Cosited
            || desc.color.chroma_siting.y == crate::interop::ChromaOffset::Cosited)
            && !format_features.contains(vk::FormatFeatureFlags::COSITED_CHROMA_SAMPLES)
        {
            return Err(NativeError::Unsupported(
                "cosited chroma is not a feature of this buffer's format",
            ));
        }
    }

    let dev = &shared.vk.device;
    let mut ext_format = vk::ExternalFormatANDROID::default();
    if external {
        ext_format = ext_format.external_format(external_id);
    }
    let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
    let mut create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk_format)
        .extent(vk::Extent3D {
            width: ahb_desc.width,
            height: ahb_desc.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external_info);
    if external {
        create_info = create_info.push_next(&mut ext_format);
    }
    let image = unsafe { dev.create_image(&create_info, None) }.map_err(NativeError::from)?;
    let result = bind_and_finish(
        shared,
        image,
        buffer,
        desc,
        ahb_desc,
        AhbReport {
            allocation_size,
            memory_type_bits,
            vk_format,
            external_id,
            required_mapping,
        },
        external,
    );
    match result {
        Ok(frame) => Ok(frame),
        Err(err) => {
            unsafe { dev.destroy_image(image, None) };
            Err(err)
        }
    }
}

/// The fields `get_android_hardware_buffer_properties` reported,
/// detached from the chained query structs.
#[derive(Clone, Copy)]
struct AhbReport {
    allocation_size: u64,
    memory_type_bits: u32,
    vk_format: vk::Format,
    external_id: u64,
    required_mapping: vk::ComponentMapping,
}

/// Imports the buffer's memory and finishes the generation record.
#[expect(clippy::too_many_lines)]
fn bind_and_finish(
    shared: &Arc<Shared>,
    image: vk::Image,
    buffer: *mut ndk_sys::AHardwareBuffer,
    mut desc: Ahb,
    ahb_desc: ndk_sys::AHardwareBuffer_Desc,
    report: AhbReport,
    external: bool,
) -> Result<Frame, NativeError> {
    let dev = &shared.vk.device;

    let mut dedicated = vk::MemoryDedicatedRequirements::default();
    let info = vk::ImageMemoryRequirementsInfo2::default().image(image);
    let mut reqs2 = vk::MemoryRequirements2::default().push_next(&mut dedicated);
    unsafe { dev.get_image_memory_requirements2(&info, &mut reqs2) };
    let reqs = reqs2.memory_requirements;

    let mut import_ahb =
        vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(buffer.cast());
    let mut dedicated_alloc = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let mut alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(report.allocation_size)
        // The import must honour both the buffer's and the image's type
        // mask; an empty intersection cannot satisfy the import.
        .memory_type_index((report.memory_type_bits & reqs.memory_type_bits).trailing_zeros())
        .push_next(&mut import_ahb);
    // Dedicated-allocation requirements are part of the import contract.
    if dedicated.requires_dedicated_allocation == vk::TRUE
        || dedicated.prefers_dedicated_allocation == vk::TRUE
    {
        alloc = alloc.push_next(&mut dedicated_alloc);
    }
    let memory = unsafe { dev.allocate_memory(&alloc, None) }?;
    if let Err(err) = unsafe { dev.bind_image_memory(image, memory, 0) } {
        unsafe { dev.free_memory(memory, None) };
        return Err(err.into());
    }

    let (repr, views, conv, rgb_wrap, pool, view_handles) = if external {
        // The external-format path: a conversion created for the buffer's
        // external-format id with the frame's declared contract, a view
        // carrying it, and the immutable combined-sampler layout.
        let key = ycbcr::ConvKey {
            format: vk::Format::UNDEFINED,
            external_format: report.external_id,
            model: required_model(&desc.color),
            range: required_range(&desc.color),
            mapping: {
                let m = report.required_mapping;
                [m.r, m.g, m.b, m.a]
            },
            chroma_x: chroma_location(desc.color.chroma_siting.x),
            chroma_y: chroma_location(desc.color.chroma_siting.y),
            filter: vk::Filter::NEAREST,
        };
        let conv = ycbcr::get(shared, key)?;
        let mut conv_info = vk::SamplerYcbcrConversionInfo::default().conversion(conv.conversion);
        let view = unsafe {
            dev.create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(vk::Format::UNDEFINED)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .push_next(&mut conv_info),
                None,
            )
        }
        .map_err(NativeError::from)?;
        let pool = create_pool(shared, true)?;
        (
            super::Repr::ExternalFormat {
                id: report.external_id,
            },
            sync::Views::ExternalFormat { view },
            Some(conv),
            None,
            Some(pool),
            vec![view],
        )
    } else {
        // A known-format buffer: RGBA8/RGBA16F single-plane buffers wrap
        // as wgpu textures; any other known Vulkan format is outside the
        // frame contract.
        let wgpu_format = vk_format_of(report.vk_format);
        match wgpu_format {
            Some(wgpu_format) => {
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
                                width: ahb_desc.width,
                                height: ahb_desc.height,
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: wgpu_format,
                            usage: wgpu::wgt::TextureUses::RESOURCE,
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
                                label: Some("external frame"),
                                size: wgpu::Extent3d {
                                    width: ahb_desc.width,
                                    height: ahb_desc.height,
                                    depth_or_array_layers: 1,
                                },
                                mip_level_count: 1,
                                sample_count: 1,
                                dimension: wgpu::TextureDimension::D2,
                                format: wgpu_format,
                                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                                view_formats: &[],
                            },
                            wgpu::wgt::TextureUses::RESOURCE,
                        )
                };
                (
                    super::Repr::Rgb {
                        format: wgpu_format,
                    },
                    sync::Views::Wrapped,
                    None,
                    Some(texture),
                    None,
                    Vec::new(),
                )
            }
            None => {
                return Err(NativeError::Unsupported(
                    "AHB's Vulkan format is not one the frame contract covers",
                ));
            }
        }
    };

    let plane = super::PlaneSource {
        buffer: std::ptr::NonNull::new(buffer).expect("checked at import"),
        acquire: match &desc.sync {
            None => super::PlaneAcquire::Ready,
            Some(super::Wait::SyncFd { fd }) => super::PlaneAcquire::Fence(
                fd.try_clone()
                    .map_err(|_| NativeError::Invalid("the acquire fence cannot be duplicated"))?,
            ),
            Some(_) => super::PlaneAcquire::Semaphore,
        },
        overlay: ahb_desc.usage
            & ndk_sys::AHardwareBuffer_UsageFlags::AHARDWAREBUFFER_USAGE_COMPOSER_OVERLAY.0
            != 0,
        hdr: desc.hdr,
    };

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
            Some(unsafe { dev.create_semaphore(&info, None) }.map_err(NativeError::from)?)
        }
        _ => None,
    };

    Ok(Frame {
        generation: Arc::new(sync::Generation {
            shared: Arc::clone(shared),
            size: (ahb_desc.width, ahb_desc.height),
            color: desc.color,
            alpha: desc.alpha,
            repr,
            image,
            views,
            conv,
            rgb_wrap,
            bytes: report.allocation_size,
            producer_layout: vk::ImageLayout::UNDEFINED,
            producer_family: QueueFamily::Foreign,
            aspects: vk::ImageAspectFlags::COLOR,
            pool,
            sets: std::sync::Mutex::new(FxHashMap::default()),
            state: Arc::new(std::sync::Mutex::new(sync::State::Registered)),
            wait: std::sync::Mutex::new(desc.sync.take()),
            resolved_wait: std::sync::Mutex::new(None),
            release_sync: std::sync::Mutex::new(desc.release.take()),
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
                memory: vec![memory],
                semaphores: Vec::new(),
                sync_payload: None,
                fence_semaphore: None,
                fence_fd: None,
                submitted_flag: None,
                state: None,
                acquired: false,
                producer_layout: vk::ImageLayout::UNDEFINED,
                producer_family: QueueFamily::Foreign,
                aspects: vk::ImageAspectFlags::COLOR,
                lease: sync::Lease::Ahb(buffer),
                plane_fences: Vec::new(),
            })),
            plane: Some(plane),
            plane_fences: std::sync::Mutex::new(Vec::new()),
        }),
    })
}

/// The `wgpu` format for a known-format AHB, when it maps to a single RGB
/// plane the frame contract accepts.
const fn vk_format_of(format: vk::Format) -> Option<wgpu::TextureFormat> {
    Some(match format {
        vk::Format::R8G8B8A8_UNORM => wgpu::TextureFormat::Rgba8Unorm,
        vk::Format::B8G8R8A8_UNORM => wgpu::TextureFormat::Bgra8Unorm,
        vk::Format::R16G16B16A16_SFLOAT => wgpu::TextureFormat::Rgba16Float,
        _ => return None,
    })
}
