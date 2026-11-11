//! The device memory a texture commits, known before it is created.
//!
//! A driver lays a texture out in its own tiling, compression and alignment,
//! so the bytes it commits are not the texel count times the texel size: on
//! Mali, a picture image with its mip chain commits about a third more. A
//! budget that admits optional textures counts what the driver will commit,
//! which these functions ask the driver for. Backends that publish no such
//! query (DX12 through wgpu, GL, WebGPU) count texel storage.
//!
//! The query mirrors the native texture wgpu creates for a descriptor, so it
//! covers only the descriptors the engine sizes before creating them: 2D,
//! single-sampled, one layer, renderable colour, no view formats. Anything
//! else is a caller bug and panics.

#[cfg(not(any(target_vendor = "apple", all(unix, not(target_vendor = "apple")))))]
use super::image_storage::storage_bytes;

/// Panics unless the native mirror below covers `desc`. A renderable
/// colour texture is initialized by a render pass, so wgpu adds no
/// initialization usage the mirror would miss.
fn assert_mirrored(desc: &wgpu::TextureDescriptor<'_>) {
    assert!(
        desc.dimension == wgpu::TextureDimension::D2
            && desc.sample_count == 1
            && desc.size.depth_or_array_layers == 1
            && desc.view_formats.is_empty()
            && desc.usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            && !desc.format.has_depth_aspect(),
        "the resident-size query mirrors only single-layer renderable colour 2D textures: {desc:?}"
    );
}

/// Texel storage of every mip level of `desc`, in whole blocks.
#[cfg(not(any(target_vendor = "apple", all(unix, not(target_vendor = "apple")))))]
fn texel_bytes(desc: &wgpu::TextureDescriptor<'_>) -> u64 {
    (0..desc.mip_level_count)
        .map(|level| {
            storage_bytes(
                desc.format,
                (desc.size.width >> level).max(1),
                (desc.size.height >> level).max(1),
            )
        })
        .sum()
}

/// The bytes the driver commits for a texture created from `desc`.
///
/// # Errors
/// [`RenderError::Render`](cherenkov::RenderError::Render) when the driver
/// refuses the memoryless image the Vulkan query creates.
#[cfg(all(unix, not(target_vendor = "apple")))]
pub fn texture_bytes(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
) -> Result<u64, cherenkov::RenderError> {
    use ash::vk;
    assert_mirrored(desc);
    let format = match desc.format {
        wgpu::TextureFormat::Rgba8Unorm => vk::Format::R8G8B8A8_UNORM,
        wgpu::TextureFormat::Rgba8UnormSrgb => vk::Format::R8G8B8A8_SRGB,
        wgpu::TextureFormat::Rgba16Float => vk::Format::R16G16B16A16_SFLOAT,
        other => panic!("no Vulkan resident-size mirror for {other:?}"),
    };
    // wgpu-hal's `map_texture_usage` for the usage wgpu-core passes down.
    let mut usage = vk::ImageUsageFlags::COLOR_ATTACHMENT;
    for (wgpu_usage, vk_usage) in [
        (
            wgpu::TextureUsages::COPY_SRC,
            vk::ImageUsageFlags::TRANSFER_SRC,
        ),
        (
            wgpu::TextureUsages::COPY_DST,
            vk::ImageUsageFlags::TRANSFER_DST,
        ),
        (
            wgpu::TextureUsages::TEXTURE_BINDING,
            vk::ImageUsageFlags::SAMPLED,
        ),
        (
            wgpu::TextureUsages::STORAGE_BINDING,
            vk::ImageUsageFlags::STORAGE,
        ),
        (
            wgpu::TextureUsages::TRANSIENT_ATTACHMENT,
            vk::ImageUsageFlags::TRANSIENT_ATTACHMENT,
        ),
    ] {
        if desc.usage.contains(wgpu_usage) {
            usage |= vk_usage;
        }
    }
    // wgpu-hal's `create_image_without_memory` for such a descriptor.
    let info = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(format)
        .extent(vk::Extent3D {
            width: desc.size.width,
            height: desc.size.height,
            depth: 1,
        })
        .mip_levels(desc.mip_level_count)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(usage)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED);
    // SAFETY: the engine's device is Vulkan on this target, borrowed for
    // this scope.
    let hal =
        unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }.expect("the engine device is Vulkan");
    let raw = hal.raw_device();
    // SAFETY: `info` is a complete single-plane 2D image create info; the
    // image is never bound to memory and is destroyed below.
    let image = unsafe { raw.create_image(&info, None) }.map_err(|error| {
        cherenkov::RenderError::Render(format!("resident-size query image: {error}"))
    })?;
    // SAFETY: `image` is the live image just created on `raw`.
    let bytes = unsafe { raw.get_image_memory_requirements(image) }.size;
    // SAFETY: `image` was created above, holds no memory and is unused.
    unsafe { raw.destroy_image(image, None) };
    Ok(bytes)
}

/// The bytes the driver commits for a texture created from `desc`.
///
/// # Errors
/// Never on Metal; the signature matches the Vulkan query's.
#[cfg(target_vendor = "apple")]
#[expect(
    clippy::unnecessary_wraps,
    reason = "one signature for every backend's query"
)]
pub fn texture_bytes(
    device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
) -> Result<u64, cherenkov::RenderError> {
    use objc2_metal::{
        MTLDevice as _, MTLPixelFormat, MTLStorageMode, MTLTextureDescriptor, MTLTextureType,
        MTLTextureUsage,
    };
    assert_mirrored(desc);
    let format = match desc.format {
        wgpu::TextureFormat::Rgba8Unorm => MTLPixelFormat::RGBA8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb => MTLPixelFormat::RGBA8Unorm_sRGB,
        wgpu::TextureFormat::Rgba16Float => MTLPixelFormat::RGBA16Float,
        other => panic!("no Metal resident-size mirror for {other:?}"),
    };
    // wgpu-hal's `map_texture_usage`: copies need no Metal usage.
    let mut usage = MTLTextureUsage::RenderTarget;
    if desc.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING) {
        usage |= MTLTextureUsage::ShaderRead;
    }
    if desc.usage.contains(wgpu::TextureUsages::STORAGE_BINDING) {
        usage |= MTLTextureUsage::ShaderRead | MTLTextureUsage::ShaderWrite;
    }
    // wgpu-hal's `create_texture` for such a descriptor.
    let descriptor = MTLTextureDescriptor::new();
    descriptor.setTextureType(MTLTextureType::Type2D);
    // SAFETY: the extent and level count are the descriptor's, which wgpu
    // validates against the device limits before creating it.
    unsafe {
        descriptor.setWidth(desc.size.width as usize);
        descriptor.setHeight(desc.size.height as usize);
        descriptor.setMipmapLevelCount(desc.mip_level_count as usize);
    }
    descriptor.setPixelFormat(format);
    descriptor.setUsage(usage);
    descriptor.setStorageMode(
        if desc
            .usage
            .contains(wgpu::TextureUsages::TRANSIENT_ATTACHMENT)
        {
            MTLStorageMode::Memoryless
        } else {
            MTLStorageMode::Private
        },
    );
    // SAFETY: Metal is the engine's backend on Apple targets.
    let hal = unsafe { device.as_hal::<wgpu::hal::metal::Api>() }.expect("Metal device");
    let heap = hal
        .raw_device()
        .heapTextureSizeAndAlignWithDescriptor(&descriptor);
    // A texture outside a heap occupies whole virtual-memory pages.
    Ok(objc2_foundation::NSRoundUpToMultipleOfPageSize(heap.size) as u64)
}

/// The bytes the driver commits for a texture created from `desc`: its
/// texel storage, the only size this backend publishes through wgpu.
///
/// # Errors
/// Never; the signature matches the Vulkan query's.
#[cfg(not(any(target_vendor = "apple", all(unix, not(target_vendor = "apple")))))]
#[expect(
    clippy::unnecessary_wraps,
    reason = "one signature for every backend's query"
)]
pub fn texture_bytes(
    _device: &wgpu::Device,
    desc: &wgpu::TextureDescriptor<'_>,
) -> Result<u64, cherenkov::RenderError> {
    assert_mirrored(desc);
    Ok(texel_bytes(desc))
}

/// The bytes the driver committed for `texture`.
#[cfg(all(unix, not(target_vendor = "apple")))]
pub fn committed_bytes(device: &wgpu::Device, texture: &wgpu::Texture) -> u64 {
    // SAFETY: the engine's device is Vulkan on this target, borrowed for
    // this scope.
    let hal =
        unsafe { device.as_hal::<wgpu::hal::vulkan::Api>() }.expect("the engine device is Vulkan");
    let image = super::raw_vk_image(texture);
    // SAFETY: `image` is `texture`'s live image on this device.
    unsafe { hal.raw_device().get_image_memory_requirements(image) }.size
}

/// The bytes the driver committed for `texture`.
#[cfg(target_vendor = "apple")]
pub fn committed_bytes(_device: &wgpu::Device, texture: &wgpu::Texture) -> u64 {
    use objc2_metal::MTLResource as _;
    // SAFETY: Metal is the engine's backend on Apple targets, and the
    // native texture is only read while its guard lives.
    let native = unsafe { texture.as_hal::<wgpu::hal::metal::Api>() }.expect("Metal texture");
    native.raw_handle().allocatedSize() as u64
}

/// The bytes the driver committed for `texture`: its texel storage.
#[cfg(not(any(target_vendor = "apple", all(unix, not(target_vendor = "apple")))))]
pub fn committed_bytes(_device: &wgpu::Device, texture: &wgpu::Texture) -> u64 {
    texel_bytes(&wgpu::TextureDescriptor {
        label: None,
        size: texture.size(),
        mip_level_count: texture.mip_level_count(),
        sample_count: texture.sample_count(),
        dimension: texture.dimension(),
        format: texture.format(),
        usage: texture.usage(),
        view_formats: &[],
    })
}
