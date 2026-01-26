//! Android AHardwareBuffer import helpers (Vulkan / wgpu-hal).
//!
//! This module intentionally avoids calling `AHardwareBuffer_*` APIs from Rust, because the
//! Android runtime targets `minSdk < 26` and direct linking would break on older devices.
//!
//! Instead, callers pass:
//! - a raw `AHardwareBuffer*` (as `*mut c_void`)
//! - a drop callback (function pointer) that releases the acquired native reference
//!   (implemented in JNI/C++ using runtime symbol lookup)
//!
//! The drop callback is invoked from wgpu-hal's `DropCallback` when the imported texture is no
//! longer used by wgpu (after GPU work completes), ensuring correct lifetime for zero-copy imports.

#![cfg(target_os = "android")]

use core::ffi::c_void;

use ash::vk;
use wgpu_hal::api::Vulkan as VulkanApi;
use wgpu_hal::Api as _;

pub type ExternalDropFn = unsafe extern "C" fn(user_data: *mut c_void);

fn vk_to_wgpu_format(format: vk::Format) -> Option<wgpu::TextureFormat> {
    Some(match format {
        vk::Format::R8G8B8A8_UNORM => wgpu::TextureFormat::Rgba8Unorm,
        vk::Format::R8G8B8A8_SRGB => wgpu::TextureFormat::Rgba8UnormSrgb,
        vk::Format::B8G8R8A8_UNORM => wgpu::TextureFormat::Bgra8Unorm,
        vk::Format::B8G8R8A8_SRGB => wgpu::TextureFormat::Bgra8UnormSrgb,
        vk::Format::R16G16B16A16_SFLOAT => wgpu::TextureFormat::Rgba16Float,
        vk::Format::A2B10G10R10_UNORM_PACK32 => wgpu::TextureFormat::Rgb10a2Unorm,
        _ => return None,
    })
}

fn find_memory_type_index(
    mem_properties: &vk::PhysicalDeviceMemoryProperties,
    type_bits: u32,
    required_flags: vk::MemoryPropertyFlags,
) -> Option<u32> {
    for i in 0..mem_properties.memory_type_count {
        let type_supported = (type_bits & (1 << i)) != 0;
        let properties_match = mem_properties.memory_types[i as usize]
            .property_flags
            .contains(required_flags);

        if type_supported && properties_match {
            return Some(i);
        }
    }
    None
}

pub(crate) fn import_ahardwarebuffer_as_wgpu_texture(
    device: &wgpu::Device,
    ahb_ptr: *mut c_void,
    width: u32,
    height: u32,
    label: &'static str,
    ahb_drop: ExternalDropFn,
    ahb_drop_data: *mut c_void,
) -> Result<(wgpu::Texture, wgpu::TextureFormat), &'static str> {
    if ahb_ptr.is_null() {
        return Err("AHardwareBuffer pointer is null");
    }
    if width == 0 || height == 0 {
        return Err("AHardwareBuffer import requires non-zero dimensions");
    }

    let Some(imported) = unsafe {
        device.as_hal::<VulkanApi, _, _>(|hal_device| {
            hal_device.map(|hal_device| {
                import_ahardwarebuffer_as_hal_texture(
                    hal_device,
                    ahb_ptr,
                    width,
                    height,
                    label,
                    ahb_drop,
                    ahb_drop_data,
                )
            })
        })
    } else {
        return Err("AHardwareBuffer import requires Vulkan backend");
    };

    let (hal_texture, format) = imported?;

    let desc = wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    };

    // SAFETY: `hal_texture` is created from a valid VkImage with external memory imported, and
    // it remains alive until wgpu drops the texture (see DropCallback).
    let wgpu_texture = unsafe { device.create_texture_from_hal::<VulkanApi>(hal_texture, &desc) };

    Ok((wgpu_texture, format))
}

fn import_ahardwarebuffer_as_hal_texture(
    hal_device: &wgpu_hal::vulkan::Device,
    ahb_ptr: *mut c_void,
    width: u32,
    height: u32,
    label: &'static str,
    ahb_drop: ExternalDropFn,
    ahb_drop_data: *mut c_void,
) -> Result<(wgpu_hal::vulkan::Texture, wgpu::TextureFormat), &'static str> {
    let raw_device = hal_device.raw_device();
    let shared = hal_device.shared_instance();
    let physical_device = shared.physical_device();
    let instance = shared.raw_instance();

    let ext = ash::android::external_memory_android_hardware_buffer::Device::new(
        instance,
        raw_device,
    );

    let mut format_props = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
    let mut ahb_props =
        vk::AndroidHardwareBufferPropertiesANDROID::default().push_next(&mut format_props);

    unsafe {
        ext.get_android_hardware_buffer_properties(ahb_ptr.cast(), &mut ahb_props)
            .map_err(|_| "vkGetAndroidHardwareBufferPropertiesANDROID failed")?;
    }

    if format_props.external_format != 0 {
        return Err("AHardwareBuffer uses external format (YUV/etc), not supported for import");
    }

    let vk_format = format_props.format;
    let Some(wgpu_format) = vk_to_wgpu_format(vk_format) else {
        return Err("Unsupported AHardwareBuffer Vulkan format for import");
    };

    // Ensure the format can be sampled.
    if !format_props
        .format_features
        .contains(vk::FormatFeatureFlags::SAMPLED_IMAGE)
    {
        return Err("AHardwareBuffer format does not support sampled image usage");
    }

    let mut external_memory_create_info = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);

    let image_create_info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(vk_format)
        .extent(vk::Extent3D {
            width,
            height,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::SAMPLED)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external_memory_create_info);

    let vk_image = unsafe {
        raw_device
            .create_image(&image_create_info, None)
            .map_err(|_| "Failed to create VkImage for AHardwareBuffer import")?
    };

    let memory_properties = unsafe { instance.get_physical_device_memory_properties(physical_device) };

    let memory_type_index = find_memory_type_index(
        &memory_properties,
        ahb_props.memory_type_bits,
        vk::MemoryPropertyFlags::empty(),
    )
    .ok_or("No suitable Vulkan memory type for AHardwareBuffer import")?;

    let mut dedicated_allocate_info = vk::MemoryDedicatedAllocateInfo::default().image(vk_image);
    let mut import_info =
        vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(ahb_ptr.cast());

    let allocate_info = vk::MemoryAllocateInfo::default()
        .allocation_size(ahb_props.allocation_size)
        .memory_type_index(memory_type_index)
        .push_next(&mut dedicated_allocate_info)
        .push_next(&mut import_info);

    let vk_memory = unsafe {
        raw_device
            .allocate_memory(&allocate_info, None)
            .map_err(|_| {
                raw_device.destroy_image(vk_image, None);
                "Failed to allocate Vulkan memory for AHardwareBuffer import"
            })?
    };

    unsafe {
        raw_device
            .bind_image_memory(vk_image, vk_memory, 0)
            .map_err(|_| {
                raw_device.free_memory(vk_memory, None);
                raw_device.destroy_image(vk_image, None);
                "Failed to bind Vulkan memory for AHardwareBuffer import"
            })?;
    }

    let raw_device_for_drop = raw_device.clone();
    let drop_callback: wgpu_hal::DropCallback = Box::new(move || unsafe {
        raw_device_for_drop.destroy_image(vk_image, None);
        raw_device_for_drop.free_memory(vk_memory, None);
        ahb_drop(ahb_drop_data);
    });

    let hal_desc = wgpu_hal::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu_format,
        usage: wgpu::wgt::TextureUses::RESOURCE,
        memory_flags: wgpu_hal::MemoryFlags::empty(),
        view_formats: alloc::vec::Vec::new(),
    };

    let hal_texture = unsafe { hal_device.texture_from_raw(vk_image, &hal_desc, Some(drop_callback)) };

    Ok((hal_texture, wgpu_format))
}
