//! DMA-BUF import through Vulkan external memory.

use std::os::fd::{AsRawFd as _, IntoRawFd as _};

use ash::vk;

use super::frame::{DRM_FORMAT_MOD_INVALID, DmaBufFormat, DmaBufFrame, DmaBufPlane};

/// A `VkImage` aliasing an imported DMA-BUF, alive until the GPU work reading
/// it has completed.
pub(super) struct ImportedVulkanImage {
    device: ash::Device,
    image: vk::Image,
    memory: vk::DeviceMemory,
    queue_family_index: u32,
}

impl core::fmt::Debug for ImportedVulkanImage {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ImportedVulkanImage")
            .finish_non_exhaustive()
    }
}

// SAFETY: `ash::Device` is not `Send` only because it wraps the dispatchable
// `VkDevice` handle as a raw pointer; the image and memory fields are
// non-dispatchable `u64` handles. Vulkan permits a `VkDevice` to be used from
// any thread, and the two calls this type makes off-thread —
// `vkDestroyImage` and `vkFreeMemory` in `Drop` — require external
// synchronization only on the objects they destroy. This type owns its image
// and memory outright: they are created in `import_dma_buf`, never handed out
// or cloned, and destroyed exactly once here, so no other thread can name them.
// `Send` is what lets a deferred import be moved into wgpu's
// `on_submitted_work_done` callback, which is where it must be dropped: that is
// the point at which the GPU has finished the copy that reads the image.
unsafe impl Send for ImportedVulkanImage {}

impl ImportedVulkanImage {
    pub(super) fn record_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        destination: &wgpu::Texture,
        width: u32,
        height: u32,
    ) {
        // SAFETY: `Texture::as_hal` requires naming the backend the texture
        // actually has and leaving wgpu's own view of it intact. The backend is
        // checked rather than assumed — the importer selects the Vulkan path
        // only for `wgpu::Backend::Vulkan`, and a mismatch yields `None` and
        // panics here. The guard is used only to read the handle, and it is
        // still alive at that point because it is bound to a local.
        let destination = unsafe {
            destination
                .as_hal::<wgpu::hal::api::Vulkan>()
                .expect("DMA-BUF destination texture is not Vulkan")
        };
        // SAFETY: `raw_handle` exposes the underlying `VkImage`, valid for as
        // long as the wgpu texture it came from lives. The caller holds that
        // texture by reference across this whole function, and the handle is
        // only recorded into a command buffer that is submitted before the
        // borrow ends.
        let destination = unsafe { destination.raw_handle() };
        // SAFETY: recording raw Vulkan into a wgpu encoder. `as_hal_mut` needs
        // the right backend, which is checked as above and panics on `None`,
        // and requires that the commands recorded leave the encoder in a state
        // wgpu can keep using. This block records only pipeline barriers and one
        // `vkCmdCopyImage`; it starts and ends no render pass and allocates no
        // resources, so the encoder is exactly where wgpu left it afterwards.
        // The two barriers form a matched pair that acquires `self.image` from
        // `QUEUE_FAMILY_EXTERNAL` into this device's queue family and releases
        // it back, which is what the DMA-BUF's external ownership requires; the
        // layouts they name match the ones `vkCmdCopyImage` is given. The
        // destination is transitioned by wgpu itself, which knows it as a
        // `COPY_DST` texture. Extents come from the frame the image was imported
        // at, so the copy stays inside both images.
        unsafe {
            encoder.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|encoder| {
                let encoder = encoder.expect("DMA-BUF command encoder is not Vulkan");
                let subresource_range = vk::ImageSubresourceRange::default()
                    .aspect_mask(vk::ImageAspectFlags::COLOR)
                    .base_mip_level(0)
                    .level_count(1)
                    .base_array_layer(0)
                    .layer_count(1);
                let acquire = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::MEMORY_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .old_layout(vk::ImageLayout::GENERAL)
                    .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .dst_queue_family_index(self.queue_family_index)
                    .image(self.image)
                    .subresource_range(subresource_range);
                self.device.cmd_pipeline_barrier(
                    encoder.raw_handle(),
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[acquire],
                );
                let region = vk::ImageCopy::default()
                    .src_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .dst_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .mip_level(0)
                            .base_array_layer(0)
                            .layer_count(1),
                    )
                    .extent(vk::Extent3D {
                        width,
                        height,
                        depth: 1,
                    });
                self.device.cmd_copy_image(
                    encoder.raw_handle(),
                    self.image,
                    vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                    destination,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[region],
                );
                let release = vk::ImageMemoryBarrier::default()
                    .src_access_mask(vk::AccessFlags::TRANSFER_READ)
                    .dst_access_mask(vk::AccessFlags::MEMORY_READ | vk::AccessFlags::MEMORY_WRITE)
                    .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(self.queue_family_index)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_EXTERNAL)
                    .image(self.image)
                    .subresource_range(subresource_range);
                self.device.cmd_pipeline_barrier(
                    encoder.raw_handle(),
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::ALL_COMMANDS,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[release],
                );
            });
        }
    }
}

impl Drop for ImportedVulkanImage {
    fn drop(&mut self) {
        // SAFETY: both handles were created in `import_dma_buf`, are owned
        // solely by this value, and are destroyed exactly once here. The image
        // is destroyed before the memory it is bound to, as Vulkan requires.
        // Neither may still be in use by the GPU: both paths that create an
        // `ImportedVulkanImage` guarantee this before dropping it — the
        // synchronous copy waits on the submission with `PollType::Wait`, and
        // the deferred copy defers the drop into `on_submitted_work_done`.
        // Freeing the memory also closes the DMA-BUF descriptor Vulkan took
        // ownership of at import.
        unsafe {
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
        }
    }
}

pub(super) fn import_dma_buf(
    device: &wgpu::Device,
    frame: &mut DmaBufFrame,
) -> ImportedVulkanImage {
    // SAFETY: `Device::as_hal` requires the named backend to be the device's
    // real one and that the exposed device is not used to invalidate wgpu's
    // state. The backend is checked: this function is reached only through the
    // importer's Vulkan path, which is selected for `wgpu::Backend::Vulkan`
    // alone, and a mismatch panics here instead of being reinterpreted. The raw
    // device is used only to create a new image and memory of this function's
    // own, never to touch anything wgpu owns.
    let hal_device = unsafe {
        device
            .as_hal::<wgpu::hal::api::Vulkan>()
            .expect("DMA-BUF Vulkan import requires a Vulkan device")
    };
    validate_import(&hal_device, frame.modifier);
    let raw = hal_device.raw_device();
    let queue_family_index = hal_device.queue_family_index();
    let plane = frame
        .planes
        .pop()
        .expect("a packed DMA-BUF frame must contain one plane");
    let image = create_import_image(raw, frame, &plane);
    let memory = import_image_memory(&hal_device, image, plane);
    // SAFETY: `image` and `memory` were both just created on `raw`, so they
    // belong to this device and neither has been bound before — this is the one
    // and only bind for each. `validate_import` established that the device
    // enables the external-memory and DRM-modifier extensions the pair was
    // created with. The memory was allocated from a type in
    // `vkGetImageMemoryRequirements(image).memoryTypeBits` (intersected with
    // what the descriptor supports) and sized to that requirement's `size`,
    // with a `VkMemoryDedicatedAllocateInfo` naming this exact image, so
    // offset 0 satisfies the alignment requirement by construction.
    if let Err(error) = unsafe { raw.bind_image_memory(image, memory, 0) } {
        // SAFETY: binding failed, so nothing owns these yet and neither is in
        // use by the GPU. Both are still live handles from `raw`, destroyed
        // exactly once here, memory after the image bound to it. Freeing the
        // memory closes the imported DMA-BUF descriptor Vulkan took over.
        unsafe {
            raw.free_memory(memory, None);
            raw.destroy_image(image, None);
        }
        panic!("failed to bind DMA-BUF Vulkan image memory: {error}");
    }
    ImportedVulkanImage {
        device: raw.clone(),
        image,
        memory,
        queue_family_index,
    }
}

fn validate_import(hal_device: &wgpu::hal::vulkan::Device, modifier: u64) {
    assert!(
        hal_device
            .enabled_device_extensions()
            .contains(&ash::khr::external_memory_fd::NAME),
        "Vulkan device does not enable VK_KHR_external_memory_fd"
    );
    assert!(
        hal_device
            .enabled_device_extensions()
            .contains(&ash::ext::external_memory_dma_buf::NAME),
        "Vulkan device does not enable VK_EXT_external_memory_dma_buf"
    );
    assert!(
        hal_device
            .enabled_device_extensions()
            .contains(&ash::ext::image_drm_format_modifier::NAME),
        "Vulkan device does not enable VK_EXT_image_drm_format_modifier"
    );
    assert_ne!(
        modifier, DRM_FORMAT_MOD_INVALID,
        "Vulkan DMA-BUF import requires an explicit DRM format modifier"
    );
}

fn create_import_image(raw: &ash::Device, frame: &DmaBufFrame, plane: &DmaBufPlane) -> vk::Image {
    let handle_type = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
    let mut external = vk::ExternalMemoryImageCreateInfo::default().handle_types(handle_type);
    let plane_layout = vk::SubresourceLayout {
        offset: u64::from(plane.offset),
        size: 0,
        row_pitch: u64::from(plane.stride),
        array_pitch: 0,
        depth_pitch: 0,
    };
    let mut modifier = vk::ImageDrmFormatModifierExplicitCreateInfoEXT::default()
        .drm_format_modifier(frame.modifier)
        .plane_layouts(std::slice::from_ref(&plane_layout));
    let create_info = vk::ImageCreateInfo::default()
        .push_next(&mut external)
        .push_next(&mut modifier)
        .image_type(vk::ImageType::TYPE_2D)
        .format(match frame.format {
            DmaBufFormat::Bgra8 | DmaBufFormat::Bgrx8 => vk::Format::B8G8R8A8_UNORM,
            DmaBufFormat::Rgba8 | DmaBufFormat::Rgbx8 => vk::Format::R8G8B8A8_UNORM,
        })
        .extent(vk::Extent3D {
            width: frame.width,
            height: frame.height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::DRM_FORMAT_MODIFIER_EXT)
        .usage(vk::ImageUsageFlags::TRANSFER_SRC)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    // SAFETY: `create_info` is fully initialized above and its two `push_next`
    // extension structs — `VkExternalMemoryImageCreateInfo` and
    // `VkImageDrmFormatModifierExplicitCreateInfoEXT` — are local `mut`
    // bindings that outlive this call, as the borrow checker enforces through
    // `ImageCreateInfo`'s lifetime parameter. `plane_layout` likewise outlives
    // the borrow `plane_layouts` takes of it. The extensions those structs
    // require were asserted enabled by `validate_import`, which also rejected
    // `DRM_FORMAT_MOD_INVALID`, so `DRM_FORMAT_MODIFIER_EXT` tiling has the
    // explicit modifier it demands and the single plane layout matches the
    // single-plane packed format asserted at the frame's construction.
    unsafe { raw.create_image(&create_info, None) }
        .unwrap_or_else(|error| panic!("failed to create Vulkan DMA-BUF import image: {error}"))
}

fn import_image_memory(
    hal_device: &wgpu::hal::vulkan::Device,
    image: vk::Image,
    plane: DmaBufPlane,
) -> vk::DeviceMemory {
    let raw = hal_device.raw_device();
    let handle_type = vk::ExternalMemoryHandleTypeFlags::DMA_BUF_EXT;
    // SAFETY: `image` was created on `raw` by the caller and has not been
    // destroyed, which is all `vkGetImageMemoryRequirements` requires; it only
    // reads the image and writes the returned struct.
    let requirements = unsafe { raw.get_image_memory_requirements(image) };
    let loader =
        ash::khr::external_memory_fd::Device::new(hal_device.shared_instance().raw_instance(), raw);
    let mut fd_properties = vk::MemoryFdPropertiesKHR::default();
    // SAFETY: `VK_KHR_external_memory_fd` was asserted enabled by
    // `validate_import`, so the loader's entry point exists. The descriptor is
    // borrowed from `plane`, which still owns it here, so it is open for the
    // call; the query does not consume it. `fd_properties` is an initialized
    // local the call writes through. On failure the image created by the caller
    // is destroyed before panicking — it is not yet bound to any memory and
    // nothing else refers to it.
    unsafe {
        loader
            .get_memory_fd_properties(handle_type, plane.fd.as_raw_fd(), &mut fd_properties)
            .unwrap_or_else(|error| {
                raw.destroy_image(image, None);
                panic!("failed to query DMA-BUF Vulkan memory properties: {error}")
            });
    }
    let type_bits = requirements.memory_type_bits & fd_properties.memory_type_bits;
    assert!(
        type_bits != 0,
        "the DMA-BUF is incompatible with every Vulkan memory type"
    );
    // SAFETY: the instance and physical device both come from the live hal
    // device guard the caller holds, so they are valid and belong together.
    // The query only reads them and returns a value.
    let memory_properties = unsafe {
        hal_device
            .shared_instance()
            .raw_instance()
            .get_physical_device_memory_properties(hal_device.raw_physical_device())
    };
    let memory_type_index = select_memory_type(type_bits, &memory_properties);
    let imported_fd = plane.fd.into_raw_fd();
    let mut import = vk::ImportMemoryFdInfoKHR::default()
        .handle_type(handle_type)
        .fd(imported_fd);
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let allocation = vk::MemoryAllocateInfo::default()
        .push_next(&mut import)
        .push_next(&mut dedicated)
        .allocation_size(requirements.size)
        .memory_type_index(memory_type_index);
    // SAFETY: `allocation` is fully initialized and its two `push_next` structs
    // are local `mut` bindings outliving the call. `VK_KHR_external_memory_fd`
    // and `VK_EXT_external_memory_dma_buf` were asserted enabled by
    // `validate_import`, so `DMA_BUF_EXT` is an accepted handle type.
    // `memory_type_index` was chosen from `type_bits`, the intersection of the
    // image's requirements with the types the descriptor supports, which was
    // asserted non-empty. `imported_fd` is open and, per the Vulkan spec, is
    // transferred to the implementation on success — which is why nothing
    // closes it afterwards and why `plane.fd` was consumed with `into_raw_fd`
    // rather than borrowed.
    match unsafe { raw.allocate_memory(&allocation, None) } {
        Ok(memory) => memory,
        Err(error) => {
            // SAFETY: on failure the implementation did *not* take the
            // descriptor, so this side still owns it and must close it exactly
            // once; `imported_fd` has not been closed and no `OwnedFd` holds it
            // any more, `into_raw_fd` having released it. The image is a live
            // handle from `raw`, unbound and unused by the GPU, destroyed once.
            unsafe {
                libc::close(imported_fd);
                raw.destroy_image(image, None);
            }
            panic!("failed to import the DMA-BUF as Vulkan memory: {error}");
        }
    }
}

fn select_memory_type(type_bits: u32, properties: &vk::PhysicalDeviceMemoryProperties) -> u32 {
    let mut first = None;
    for index in 0..properties.memory_type_count {
        if type_bits & (1 << index) == 0 {
            continue;
        }
        first.get_or_insert(index);
        let memory_type = properties.memory_types
            [usize::try_from(index).expect("Vulkan memory index must fit usize")];
        if memory_type
            .property_flags
            .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
        {
            return index;
        }
    }
    first.expect("a compatible Vulkan memory type disappeared")
}
