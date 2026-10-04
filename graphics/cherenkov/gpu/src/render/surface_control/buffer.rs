//! The engine's own plane buffers: `AHardwareBuffer`s the engine renders
//! into and hands to the system compositor.
//!
//! Each buffer is allocated for framebuffer, sampling and overlay use,
//! imported into the engine's `VkDevice` as a colour attachment and wrapped
//! as a `wgpu::Texture`. The wrapper's drop callback destroys the image,
//! frees the imported memory and releases the engine's buffer reference, so
//! the Vulkan objects live exactly as long as wgpu's own tracking of the
//! texture; the system compositor holds a reference of its own while it
//! shows the buffer.

use std::os::fd::OwnedFd;
use std::ptr::NonNull;
use std::sync::Arc;

use ash::vk;

use crate::render::external::vulkan::{NativeError, Shared};

/// A buffer the system compositor can take, as a thread-safe handle.
#[derive(Clone, Copy, Debug)]
pub struct Ahb(pub NonNull<ndk_sys::AHardwareBuffer>);

// SAFETY: `AHardwareBuffer` is reference-counted and its NDK entry points
// are callable from any thread.
unsafe impl Send for Ahb {}
// SAFETY: as above; the handle is never mutated through a shared reference.
unsafe impl Sync for Ahb {}

impl Ahb {
    /// The raw handle, for an NDK call.
    #[must_use]
    pub const fn as_ptr(self) -> *mut ndk_sys::AHardwareBuffer {
        self.0.as_ptr()
    }
}

/// Where a plane buffer is in its round trip through the system compositor.
#[derive(Debug)]
pub enum State {
    /// Free to render into once the fence, if any, signals.
    Free(Option<OwnedFd>),
    /// Set on a surface control; its release has not been delivered yet.
    Shown,
}

/// One engine plane buffer.
#[derive(Debug)]
pub struct Buffer {
    /// Identity carried through a transaction's completion.
    pub id: u64,
    /// The buffer handed to `ASurfaceTransaction_setBuffer`, alive while
    /// `texture` is.
    pub ahb: Ahb,
    /// The imported image the ownership barriers name.
    pub image: vk::Image,
    /// The colour attachment the presenter blits into.
    pub texture: wgpu::Texture,
    /// Imported allocation size, including the allocator's row and page padding.
    pub bytes: u64,
    /// Its round-trip state.
    pub state: State,
}

/// The format the engine's planes carry: 8-bit sRGB-encoded RGBA with
/// premultiplied alpha, as the window swapchain path presents.
pub const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Allocates a `size` buffer and wraps it as a colour attachment.
///
/// # Errors
/// [`NativeError`] when the system cannot allocate the buffer or the device
/// cannot import it.
pub fn allocate(shared: &Arc<Shared>, size: (u32, u32), id: u64) -> Result<Buffer, NativeError> {
    allocate_format(shared, size, id, FORMAT)
}

/// Allocates the negotiated attachment format, including extended linear P3.
pub fn allocate_format(
    shared: &Arc<Shared>,
    size: (u32, u32),
    id: u64,
    format: wgpu::TextureFormat,
) -> Result<Buffer, NativeError> {
    use ndk_sys::{AHardwareBuffer_Format as Format, AHardwareBuffer_UsageFlags as Usage};
    let native_format = match format {
        wgpu::TextureFormat::Rgba8Unorm => Format::AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM,
        wgpu::TextureFormat::Rgba16Float => Format::AHARDWAREBUFFER_FORMAT_R16G16B16A16_FLOAT,
        _ => unreachable!("plane attachment format"),
    };
    let Some(loader) = shared.vk.ahb.as_ref() else {
        return Err(NativeError::Unsupported(
            "VK_ANDROID_external_memory_android_hardware_buffer is not enabled",
        ));
    };
    let desc = ndk_sys::AHardwareBuffer_Desc {
        width: size.0,
        height: size.1,
        layers: 1,
        format: native_format.0,
        usage: Usage::AHARDWAREBUFFER_USAGE_GPU_FRAMEBUFFER.0
            | Usage::AHARDWAREBUFFER_USAGE_GPU_SAMPLED_IMAGE.0
            | Usage::AHARDWAREBUFFER_USAGE_COMPOSER_OVERLAY.0,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    let mut raw = core::ptr::null_mut();
    if unsafe { ndk_sys::AHardwareBuffer_allocate(&raw const desc, &raw mut raw) } != 0 {
        return Err(NativeError::Invalid("AHardwareBuffer_allocate failed"));
    }
    let ahb = Ahb(NonNull::new(raw).expect("a successful allocation returns a buffer"));
    let (image, memory, bytes) = match import(shared, loader, ahb, size, format) {
        Ok(imported) => imported,
        Err(err) => {
            unsafe { ndk_sys::AHardwareBuffer_release(raw) };
            return Err(err);
        }
    };
    let texture = wrap(shared, ahb, image, memory, size, format);
    crate::diag::create(&shared.wgpu, "plane buffer", bytes);
    Ok(Buffer {
        id,
        ahb,
        image,
        texture,
        bytes,
        state: State::Free(None),
    })
}

/// Imports `ahb` as a `size` colour-attachment image bound to its memory.
fn import(
    shared: &Shared,
    loader: &ash::android::external_memory_android_hardware_buffer::Device,
    ahb: Ahb,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> Result<(vk::Image, vk::DeviceMemory, u64), NativeError> {
    let raw = ahb.as_ptr();
    let mut props = vk::AndroidHardwareBufferPropertiesANDROID::default();
    unsafe { loader.get_android_hardware_buffer_properties(raw.cast_const().cast(), &mut props) }?;
    let dev = &shared.vk.device;
    let mut external = vk::ExternalMemoryImageCreateInfo::default()
        .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
    let create = vk::ImageCreateInfo::default()
        .image_type(vk::ImageType::TYPE_2D)
        .format(if format == FORMAT {
            vk::Format::R8G8B8A8_UNORM
        } else {
            vk::Format::R16G16B16A16_SFLOAT
        })
        .extent(vk::Extent3D {
            width: size.0,
            height: size.1,
            depth: 1,
        })
        .mip_levels(1)
        .array_layers(1)
        .samples(vk::SampleCountFlags::TYPE_1)
        .tiling(vk::ImageTiling::OPTIMAL)
        .usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
        .sharing_mode(vk::SharingMode::EXCLUSIVE)
        .initial_layout(vk::ImageLayout::UNDEFINED)
        .push_next(&mut external);
    let image = unsafe { dev.create_image(&create, None) }?;
    let requirements = unsafe { dev.get_image_memory_requirements(image) };
    let types = props.memory_type_bits & requirements.memory_type_bits;
    let mut import = vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(raw.cast());
    let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
    let alloc = vk::MemoryAllocateInfo::default()
        .allocation_size(props.allocation_size)
        .memory_type_index(types.trailing_zeros())
        .push_next(&mut import)
        .push_next(&mut dedicated);
    let bound = if types == 0 {
        Err(NativeError::Invalid(
            "no memory type imports the plane buffer",
        ))
    } else {
        unsafe { dev.allocate_memory(&alloc, None) }
            .map_err(NativeError::from)
            .and_then(
                |memory| match unsafe { dev.bind_image_memory(image, memory, 0) } {
                    Ok(()) => Ok(memory),
                    Err(err) => {
                        unsafe { dev.free_memory(memory, None) };
                        Err(err.into())
                    }
                },
            )
    };
    match bound {
        Ok(memory) => Ok((image, memory, props.allocation_size)),
        Err(err) => {
            unsafe { dev.destroy_image(image, None) };
            Err(err)
        }
    }
}

/// Wraps the imported `image` as a wgpu texture whose drop destroys it,
/// frees `memory` and releases the engine's reference to `ahb`.
fn wrap(
    shared: &Shared,
    ahb: Ahb,
    image: vk::Image,
    memory: vk::DeviceMemory,
    size: (u32, u32),
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    let owner = shared.vk.device.clone();
    let drop_callback: wgpu::hal::DropCallback = Box::new(move || unsafe {
        owner.destroy_image(image, None);
        owner.free_memory(memory, None);
        ndk_sys::AHardwareBuffer_release(ahb.as_ptr());
    });
    let extent = wgpu::Extent3d {
        width: size.0,
        height: size.1,
        depth_or_array_layers: 1,
    };
    unsafe {
        let hal_device = shared
            .wgpu
            .as_hal::<wgpu::hal::vulkan::Api>()
            .expect("the engine device is Vulkan");
        let hal_texture = hal_device.texture_from_raw(
            image,
            &wgpu::hal::TextureDescriptor {
                label: Some("plane buffer"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::wgt::TextureUses::COLOR_TARGET,
                memory_flags: wgpu::hal::MemoryFlags::empty(),
                view_formats: Vec::new(),
            },
            Some(drop_callback),
            wgpu::hal::vulkan::TextureMemory::External,
        );
        // Every use of the texture is bracketed by the realization's own
        // ownership barriers, which leave it in the colour-attachment
        // layout wgpu tracks here; the contents are cleared on each use.
        shared
            .wgpu
            .create_texture_from_hal::<wgpu::hal::vulkan::Api>(
                hal_texture,
                &wgpu::TextureDescriptor {
                    label: Some("plane buffer"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                },
                wgpu::wgt::TextureUses::COLOR_TARGET,
            )
    }
}

/// Records the ownership barriers around the engine's use of `images`:
/// `acquire` takes them from the system compositor into the colour
/// attachment layout on the engine's queue family, discarding the old
/// contents; otherwise they are handed back to the foreign family in the
/// general layout.
///
/// # Safety
/// `cb` must be a recording command buffer on `shared`'s device and every
/// image a live plane buffer.
pub unsafe fn ownership(
    shared: &Shared,
    cb: vk::CommandBuffer,
    images: &[vk::Image],
    acquire: bool,
) {
    let range = vk::ImageSubresourceRange {
        aspect_mask: vk::ImageAspectFlags::COLOR,
        base_mip_level: 0,
        level_count: 1,
        base_array_layer: 0,
        layer_count: 1,
    };
    let engine = shared.vk.queue_family;
    let barriers: Vec<_> = images
        .iter()
        .map(|&image| {
            let barrier = vk::ImageMemoryBarrier::default()
                .image(image)
                .subresource_range(range);
            if acquire {
                barrier
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
                    .dst_queue_family_index(engine)
            } else {
                barrier
                    .src_access_mask(vk::AccessFlags::COLOR_ATTACHMENT_WRITE)
                    .dst_access_mask(vk::AccessFlags::empty())
                    .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                    .new_layout(vk::ImageLayout::GENERAL)
                    .src_queue_family_index(engine)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
            }
        })
        .collect();
    let (src, dst) = if acquire {
        (
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
        )
    } else {
        (
            vk::PipelineStageFlags::COLOR_ATTACHMENT_OUTPUT,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
        )
    };
    unsafe {
        shared.vk.device.cmd_pipeline_barrier(
            cb,
            src,
            dst,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &barriers,
        );
    }
}
