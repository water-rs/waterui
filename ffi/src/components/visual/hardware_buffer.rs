//! Android view capture: `AHardwareBuffer` to Vulkan external memory to wgpu.
//!
//! Android's answer to Apple's `CARenderer`-into-an-`MTLTexture` capture is
//! `HardwareRenderer` drawing a `RenderNode` into an `ImageReader`, which hands
//! back an `AHardwareBuffer`. This module is what turns that buffer into pixels
//! the filter pipeline can sample, without ever taking them through the CPU.
//!
//! # Why the buffer is imported as a raw `VkImage` and copied
//!
//! The obvious shape — import the buffer as a `wgpu::Texture` through
//! `Device::create_texture_from_hal` and sample it directly — is wrong, and
//! quietly so. `wgpu-core` records every texture created that way with an
//! initial tracked state of `TextureUses::UNINITIALIZED`, so the first barrier
//! it emits for one is `VK_IMAGE_LAYOUT_UNDEFINED` to `SHADER_READ_ONLY_OPTIMAL`
//! with no queue-family acquire — a transition Vulkan explicitly permits an
//! implementation to satisfy by *discarding* the image's contents. On a tiler
//! that is exactly what happens, and the filter samples an empty capture.
//!
//! So the imported buffer is never a wgpu texture. It is a raw `vk::Image` bound
//! to imported memory, acquired from `VK_QUEUE_FAMILY_FOREIGN_EXT`, copied into
//! the wgpu-owned capture texture with `vkCmdCopyImage`, and released back to the
//! framework — one GPU-side copy of the subtree per captured frame, no readback.
//!
//! # How the copy stays truthful to wgpu's tracker
//!
//! The copy is recorded into a real `wgpu::CommandEncoder` through
//! [`wgpu::CommandEncoder::as_hal_mut`], not into a private command pool
//! submitted by hand. Two things fall out of that, both load-bearing:
//!
//! - The destination's layout is established by
//!   [`wgpu::CommandEncoder::transition_resources`] — wgpu's documented
//!   native-interoperability API — rather than assumed. wgpu emits the barrier
//!   from whatever state it is actually tracking (`COLOR_TARGET` on the frame
//!   after the texture was created, `RESOURCE` on every frame after the filter
//!   sampled it) into `COPY_DST`, and records `COPY_DST` as the new truth. The
//!   raw copy therefore finds the image in `TRANSFER_DST_OPTIMAL` and leaves it
//!   there, matching what wgpu believes on every frame.
//! - The submission is wgpu's own, so the [`WuiGpuCaptureFence`] handed back to
//!   the backend carries a real `wgpu::SubmissionIndex` and the existing
//!   completion machinery resolves it exactly. A separate `vkQueueSubmit` could
//!   not: Vulkan gives no guarantee that a later submission's fence implies an
//!   earlier one finished.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::ffi::c_void;

use ash::vk;
use ndk_sys::AHardwareBuffer;
use waterui_graphics::shared_context::{GpuRuntime, drain_device_before_teardown};
use wgpu_hal::api::Vulkan;

use super::capture_format::{WuiCaptureFormat, capture_buffer_format};
use super::gpu_surface::WuiGpuCaptureFence;

/// `AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM` as the plain `u32` a descriptor holds.
const BUFFER_FORMAT_RGBA_8888: u32 =
    ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_R8G8B8A8_UNORM.0;
/// `AHARDWAREBUFFER_FORMAT_R16G16B16A16_FLOAT`, likewise.
const BUFFER_FORMAT_RGBA_FP16: u32 =
    ndk_sys::AHardwareBuffer_Format::AHARDWAREBUFFER_FORMAT_R16G16B16A16_FLOAT.0;

/// The size and layout of one `AHardwareBuffer`, as the framework describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareBufferDescription {
    /// Buffer width in pixels.
    pub width: u32,
    /// Buffer height in pixels.
    pub height: u32,
    /// The buffer's pixel layout.
    pub format: WuiCaptureFormat,
}

/// Reads back the size and layout of a hardware buffer.
///
/// # Safety
///
/// `buffer` must be a live `AHardwareBuffer` for the duration of this call.
///
/// # Panics
///
/// Panics when the buffer's layout is not one `WaterUI` captures into, which means
/// the backend allocated its `ImageReader` with a format
/// [`capture_buffer_format`] never asked for.
#[must_use]
pub unsafe fn describe_hardware_buffer(buffer: *mut AHardwareBuffer) -> HardwareBufferDescription {
    assert!(
        !buffer.is_null(),
        "Android view capture was handed a null AHardwareBuffer"
    );
    let mut description = ndk_sys::AHardwareBuffer_Desc {
        width: 0,
        height: 0,
        layers: 0,
        format: 0,
        usage: 0,
        stride: 0,
        rfu0: 0,
        rfu1: 0,
    };
    // SAFETY: the caller contract keeps `buffer` alive for this call, and
    // `description` is writable storage for exactly one descriptor.
    unsafe { ndk_sys::AHardwareBuffer_describe(buffer, &raw mut description) };
    let format = match description.format {
        BUFFER_FORMAT_RGBA_8888 => WuiCaptureFormat::Rgba8Unorm,
        BUFFER_FORMAT_RGBA_FP16 => WuiCaptureFormat::Rgba16Float,
        other => panic!(
            "Android view capture received an AHardwareBuffer in format {other}, which is neither \
             RGBA_8888 nor RGBA_FP16"
        ),
    };
    HardwareBufferDescription {
        width: description.width,
        height: description.height,
        format,
    }
}

/// Reads the `AHardwareBuffer` behind a Java `android.hardware.HardwareBuffer`.
///
/// The returned pointer is borrowed from the Java object and is only valid while
/// that object is alive and open; an import takes its own reference before
/// keeping it.
///
/// # Safety
///
/// `env` and `hardware_buffer` must be the raw environment and the local
/// reference of the JNI call currently on this thread.
///
/// # Panics
///
/// Panics when the object is not a `HardwareBuffer`, or has been closed.
#[must_use]
pub unsafe fn hardware_buffer_from_java(
    env: *mut c_void,
    hardware_buffer: *mut c_void,
) -> *mut AHardwareBuffer {
    // SAFETY: the caller contract makes both arguments valid for this call, which
    // is all `AHardwareBuffer_fromHardwareBuffer` reads.
    let buffer =
        unsafe { ndk_sys::AHardwareBuffer_fromHardwareBuffer(env.cast(), hardware_buffer.cast()) };
    assert!(
        !buffer.is_null(),
        "Android view capture was handed a HardwareBuffer that is not backed by an AHardwareBuffer"
    );
    buffer
}

/// The whole of a 2D colour image, as every barrier here addresses it.
const COLOR_SUBRESOURCE: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};

/// The same subresource, spelled the way `vkCmdCopyImage` wants it.
const COLOR_LAYERS: vk::ImageSubresourceLayers = vk::ImageSubresourceLayers {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    mip_level: 0,
    base_array_layer: 0,
    layer_count: 1,
};

/// How many distinct buffers one capture target keeps imported at a time.
///
/// An `ImageReader` rotates a small fixed set of buffers, so the same handful of
/// pointers come back frame after frame and importing each once is the whole of
/// the caching story. This cap is well above any `maxImages` a capture would be
/// configured with; reaching it means the backend is rotating more buffers than
/// expected, and the least recently used import is evicted rather than letting
/// the list grow without bound.
const MAX_CACHED_IMPORTS: usize = 8;

/// One `AHardwareBuffer` imported as a raw Vulkan image.
///
/// The import holds a reference on the buffer for as long as it lives, so the
/// pointer it is keyed by stays valid and cannot be reused for a different
/// allocation underneath the cache.
struct ImportedHardwareBuffer {
    /// The device the image and its memory belong to. `ash::Device` is a handle
    /// plus a function table, so this clone costs nothing and lets [`Drop`]
    /// destroy them without reaching back through wgpu.
    device: ash::Device,
    /// The acquired buffer, released when this import is dropped.
    buffer: *mut AHardwareBuffer,
    /// The image bound to the buffer's imported memory.
    image: vk::Image,
    /// The imported memory the image is bound to.
    memory: vk::DeviceMemory,
    /// What the buffer was when it was imported. A buffer whose description no
    /// longer matches is a different capture and gets a fresh import.
    description: HardwareBufferDescription,
}

impl Drop for ImportedHardwareBuffer {
    fn drop(&mut self) {
        // SAFETY: both handles were created by this device in `import`, are owned
        // solely by this value, and every path that drops an import has drained
        // the device first, so no submission still reads them. `Drop` runs once,
        // so each is destroyed once.
        unsafe {
            self.device.destroy_image(self.image, None);
            self.device.free_memory(self.memory, None);
            ndk_sys::AHardwareBuffer_release(self.buffer);
        }
    }
}

impl ImportedHardwareBuffer {
    /// Imports `buffer` as a `VkImage` bound to its memory.
    ///
    /// # Safety
    ///
    /// `buffer` must be a live `AHardwareBuffer` for the duration of this call.
    ///
    /// # Panics
    ///
    /// Panics when the device was opened without the import extension, when the
    /// driver refuses the buffer, or when image creation, allocation or binding
    /// fails.
    unsafe fn import(
        device: &wgpu_hal::vulkan::Device,
        buffer: *mut AHardwareBuffer,
        description: HardwareBufferDescription,
        context: &'static str,
    ) -> Self {
        let raw_device = device.raw_device();
        let instance = device.shared_instance().raw_instance();
        let extension = ash::android::external_memory_android_hardware_buffer::NAME;
        assert!(
            device.enabled_device_extensions().contains(&extension),
            "{context}: the GPU device was opened without {}, so a captured view subtree cannot \
             be imported",
            extension.to_string_lossy()
        );
        let external_memory = ash::android::external_memory_android_hardware_buffer::Device::new(
            instance, raw_device,
        );

        let mut format_properties = vk::AndroidHardwareBufferFormatPropertiesANDROID::default();
        let (memory_type_bits, allocation_size) = {
            let mut properties = vk::AndroidHardwareBufferPropertiesANDROID::default()
                .push_next(&mut format_properties);
            // SAFETY: the caller contract keeps `buffer` alive, and `properties` is
            // writable storage chained to `format_properties` for this call only.
            unsafe {
                external_memory.get_android_hardware_buffer_properties(
                    buffer.cast::<c_void>().cast_const(),
                    &mut properties,
                )
            }
            .unwrap_or_else(|error| {
                panic!("{context}: the driver rejected the captured AHardwareBuffer: {error}")
            });
            (properties.memory_type_bits, properties.allocation_size)
        };
        // Read once the chained query above has released its borrow: the driver
        // names the format the image must be created with, and it is the only
        // format the import is allowed to claim.
        let vk_format = format_properties.format;

        let mut external_info = vk::ExternalMemoryImageCreateInfo::default()
            .handle_types(vk::ExternalMemoryHandleTypeFlags::ANDROID_HARDWARE_BUFFER_ANDROID);
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_2D)
            .format(vk_format)
            .extent(vk::Extent3D {
                width: description.width,
                height: description.height,
                depth: 1,
            })
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::TRANSFER_SRC)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED)
            .push_next(&mut external_info);
        // SAFETY: `image_info` and everything chained onto it live until this call
        // returns, and the image it creates is owned by the value built below.
        let image = unsafe { raw_device.create_image(&image_info, None) }.unwrap_or_else(|error| {
            panic!("{context}: could not create the image for a captured AHardwareBuffer: {error}")
        });

        let memory_type_index = external_memory_type_index(
            instance,
            device.raw_physical_device(),
            memory_type_bits,
            context,
        );
        let mut dedicated = vk::MemoryDedicatedAllocateInfo::default().image(image);
        let mut import_info =
            vk::ImportAndroidHardwareBufferInfoANDROID::default().buffer(buffer.cast::<c_void>());
        let allocate_info = vk::MemoryAllocateInfo::default()
            .allocation_size(allocation_size)
            .memory_type_index(memory_type_index)
            .push_next(&mut dedicated)
            .push_next(&mut import_info);
        // SAFETY: `allocate_info` and its chain live until this call returns, and
        // the buffer it imports stays alive because the caller holds it for this
        // call and the acquire below takes a reference of our own.
        let memory =
            unsafe { raw_device.allocate_memory(&allocate_info, None) }.unwrap_or_else(|error| {
                // SAFETY: `image` was created just above and nothing else owns it.
                unsafe { raw_device.destroy_image(image, None) };
                panic!(
                    "{context}: could not import the memory of a captured AHardwareBuffer: {error}"
                )
            });
        // SAFETY: the image was created for exactly this dedicated allocation, and
        // neither has been bound before.
        unsafe { raw_device.bind_image_memory(image, memory, 0) }.unwrap_or_else(|error| {
            panic!("{context}: could not bind a captured AHardwareBuffer to its image: {error}")
        });

        // The import outlives the Java `HardwareBuffer` it came from, so it holds
        // its own reference for as long as the image is bound to it.
        // SAFETY: the caller contract keeps `buffer` alive for this call, which is
        // when the reference is taken; `Drop` releases it once.
        unsafe { ndk_sys::AHardwareBuffer_acquire(buffer) };

        tracing::debug!(
            context,
            width = description.width,
            height = description.height,
            format = ?description.format,
            "imported an Android capture buffer as a Vulkan image"
        );

        Self {
            device: raw_device.clone(),
            buffer,
            image,
            memory,
            description,
        }
    }
}

/// The imports one capture target is holding.
pub struct HardwareBufferImports {
    /// The runtime whose device the imports belong to. Held so they can be
    /// destroyed after the work that reads them, including from [`Drop`], where
    /// there is no caller left to hand a device in.
    runtime: GpuRuntime,
    /// Most recently used last.
    imports: Vec<ImportedHardwareBuffer>,
}

impl Drop for HardwareBufferImports {
    fn drop(&mut self) {
        self.clear();
    }
}

impl core::fmt::Debug for HardwareBufferImports {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("HardwareBufferImports")
            .field("runtime", &self.runtime)
            .field("imports", &self.imports.len())
            .finish()
    }
}

impl HardwareBufferImports {
    /// An empty cache on `runtime`'s device, holding no buffer.
    #[must_use]
    pub const fn new(runtime: GpuRuntime) -> Self {
        Self {
            runtime,
            imports: Vec::new(),
        }
    }

    /// Releases every import, after waiting for the work that reads them.
    ///
    /// The imported images are raw Vulkan objects wgpu knows nothing about, so
    /// nothing else defers their destruction until the GPU is done with them.
    /// Called when a capture target detaches or is destroyed, which are the only
    /// moments this costs a wait — and it costs nothing when nothing is cached.
    pub fn clear(&mut self) {
        if self.imports.is_empty() {
            return;
        }
        drain_device_before_teardown(&self.runtime.context().device);
        self.imports.clear();
    }

    /// The image for `buffer`, importing it the first time it is seen.
    ///
    /// # Safety
    ///
    /// `buffer` must be a live `AHardwareBuffer` for the duration of this call.
    unsafe fn get_or_import(
        &mut self,
        device: &wgpu_hal::vulkan::Device,
        buffer: *mut AHardwareBuffer,
        description: HardwareBufferDescription,
        context: &'static str,
    ) -> vk::Image {
        if let Some(index) = self.imports.iter().position(|import| {
            core::ptr::eq(import.buffer, buffer) && import.description == description
        }) {
            let import = self.imports.remove(index);
            let image = import.image;
            self.imports.push(import);
            return image;
        }

        // A buffer whose pointer is already cached but whose description changed
        // is a different capture in the same slot, and its stale import — like any
        // import evicted for the cap — may still be read by work in flight.
        let stale = self
            .imports
            .iter()
            .position(|import| core::ptr::eq(import.buffer, buffer));
        let evicted = stale.or_else(|| (self.imports.len() >= MAX_CACHED_IMPORTS).then_some(0));
        if let Some(index) = evicted {
            drain_device_before_teardown(&self.runtime.context().device);
            drop(self.imports.remove(index));
        }

        // SAFETY: forwarding the caller's contract that `buffer` is live; `import`
        // acquires its own reference before returning.
        let import =
            unsafe { ImportedHardwareBuffer::import(device, buffer, description, context) };
        let image = import.image;
        self.imports.push(import);
        image
    }
}

/// Copies a captured hardware buffer into a wgpu texture on the GPU.
///
/// Returns the fence for the submission that performs the copy: the backend
/// registers a completion on it with `waterui_gpu_capture_fence_on_complete` and
/// closes the `Image` the buffer came from only once that fires, because until
/// then the GPU is still reading it.
///
/// # Safety
///
/// `buffer` must be a live `AHardwareBuffer` for the duration of this call.
///
/// # Panics
///
/// Panics when the buffer's size or layout does not match `destination`, when the
/// device is not the Vulkan backend, or when any Vulkan call in the import fails.
pub unsafe fn copy_hardware_buffer_into_texture(
    imports: &mut HardwareBufferImports,
    buffer: *mut AHardwareBuffer,
    destination: &wgpu::Texture,
    context: &'static str,
) -> *mut WuiGpuCaptureFence {
    // SAFETY: forwarding the caller's own contract that `buffer` is live.
    let description = unsafe { describe_hardware_buffer(buffer) };
    assert_eq!(
        (description.width, description.height),
        (destination.width(), destination.height()),
        "{context}: the captured buffer is {}x{} but the capture texture is {}x{}",
        description.width,
        description.height,
        destination.width(),
        destination.height()
    );
    assert_eq!(
        description.format,
        capture_buffer_format(destination.format()),
        "{context}: the captured buffer's layout does not match the capture texture's format"
    );

    // Cloned rather than borrowed out of `imports`, which the import below takes
    // exclusively; it is one `Arc` bump.
    let runtime = imports.runtime.clone();
    let gpu = runtime.context();
    let (raw_device, family_index, source) = {
        // SAFETY: the HAL device is only borrowed to read its raw handles and to
        // create the import's own image and memory; it is never destroyed here.
        let hal_device = unsafe { gpu.device.as_hal::<Vulkan>() }.unwrap_or_else(|| {
            panic!("{context}: Android view capture requires the Vulkan backend")
        });
        // SAFETY: forwarding the caller's contract that `buffer` is live; the
        // import acquires its own reference on it.
        let source = unsafe { imports.get_or_import(&hal_device, buffer, description, context) };
        (
            hal_device.raw_device().clone(),
            hal_device.queue_family_index(),
            source,
        )
    };

    // Scoped: the texture guard holds the device's snatchable read lock, and
    // every wgpu call below wants it too.
    let destination_image = {
        // SAFETY: the destination is a texture of this device, so its HAL type is
        // `Vulkan`; the guard only reads it.
        let guard = unsafe { destination.as_hal::<Vulkan>() }
            .unwrap_or_else(|| panic!("{context}: the capture texture is not a Vulkan texture"));
        // SAFETY: the handle is only recorded into a command buffer below, never
        // destroyed, and the texture that owns it outlives this call.
        unsafe { guard.raw_handle() }
    };

    // wgpu refuses to mix its own commands with raw HAL recording in one encoder,
    // so the frame is two: the first carries wgpu's barrier into `COPY_DST`, from
    // whatever state it is tracking, and records that new state so the layout the
    // raw commands leave the image in is the layout wgpu expects next time; the
    // second is recorded through the HAL alone. One submit keeps them in order.
    let mut transition = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("WaterUI Android Capture Transition"),
        });
    transition.transition_resources(
        core::iter::empty(),
        core::iter::once(wgpu::TextureTransition {
            texture: destination,
            selector: None,
            state: wgpu::TextureUses::COPY_DST,
        }),
    );
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("WaterUI Android Capture Copy"),
        });
    // SAFETY: the callback only records into the encoder's active command buffer
    // and never ends it, which is `as_hal_mut`'s contract, and the wgpu encoder is
    // untouched for the duration of the callback.
    unsafe {
        encoder.as_hal_mut::<Vulkan, _, ()>(|hal_encoder| {
            let hal_encoder = hal_encoder.unwrap_or_else(|| {
                panic!("{context}: the command encoder is not a Vulkan encoder")
            });
            // SAFETY: the command buffer is the encoder's own, recorded into and
            // never destroyed here.
            let command_buffer = hal_encoder.raw_handle();
            record_capture_copy(
                &raw_device,
                command_buffer,
                family_index,
                source,
                destination_image,
                description,
            );
        });
    }

    let submission = gpu.queue.submit([transition.finish(), encoder.finish()]);
    Box::into_raw(Box::new(WuiGpuCaptureFence::new(
        gpu.submission_completion_driver(),
        submission,
    )))
}

/// Records the acquire, copy and release for one captured frame.
///
/// The image arrives owned by `VK_QUEUE_FAMILY_FOREIGN_EXT` — the Android
/// framework rendered into it — and is handed straight back there afterwards so
/// the next producer can take it. An `AHardwareBuffer`-backed image keeps its
/// contents across an acquire out of `VK_IMAGE_LAYOUT_UNDEFINED`, which is what
/// makes this the standard import pattern rather than a discard.
fn record_capture_copy(
    device: &ash::Device,
    command_buffer: vk::CommandBuffer,
    family_index: u32,
    source: vk::Image,
    destination: vk::Image,
    description: HardwareBufferDescription,
) {
    let acquire = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::empty())
        .dst_access_mask(vk::AccessFlags::TRANSFER_READ)
        .old_layout(vk::ImageLayout::UNDEFINED)
        .new_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .src_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
        .dst_queue_family_index(family_index)
        .image(source)
        .subresource_range(COLOR_SUBRESOURCE);
    // SAFETY: `command_buffer` is in the recording state — wgpu is mid-encode —
    // and every handle named here belongs to `device`.
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TOP_OF_PIPE,
            vk::PipelineStageFlags::TRANSFER,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[acquire],
        );
    }

    let region = vk::ImageCopy::default()
        .src_subresource(COLOR_LAYERS)
        .dst_subresource(COLOR_LAYERS)
        .extent(vk::Extent3D {
            width: description.width,
            height: description.height,
            depth: 1,
        });
    // SAFETY: both images are in the layouts named, the region covers an extent
    // both cover, and their formats are size-compatible — they differ at most in
    // sRGB encoding, which `vkCmdCopyImage` does not interpret.
    unsafe {
        device.cmd_copy_image(
            command_buffer,
            source,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            destination,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[region],
        );
    }

    // A release may not name `VK_IMAGE_LAYOUT_UNDEFINED`, so the image is handed
    // back in `GENERAL`, which any consumer can acquire from.
    let release = vk::ImageMemoryBarrier::default()
        .src_access_mask(vk::AccessFlags::TRANSFER_READ)
        .dst_access_mask(vk::AccessFlags::empty())
        .old_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
        .new_layout(vk::ImageLayout::GENERAL)
        .src_queue_family_index(family_index)
        .dst_queue_family_index(vk::QUEUE_FAMILY_FOREIGN_EXT)
        .image(source)
        .subresource_range(COLOR_SUBRESOURCE);
    // SAFETY: as for the acquire above.
    unsafe {
        device.cmd_pipeline_barrier(
            command_buffer,
            vk::PipelineStageFlags::TRANSFER,
            vk::PipelineStageFlags::BOTTOM_OF_PIPE,
            vk::DependencyFlags::empty(),
            &[],
            &[],
            &[release],
        );
    }
}

/// A memory type that can back an imported hardware buffer.
///
/// The driver answers with a mask of every type the buffer may be imported into;
/// any of them is valid, so the lowest is taken.
///
/// # Panics
///
/// Panics when the driver reports no usable memory type, which means the buffer
/// cannot be imported on this device at all.
fn external_memory_type_index(
    instance: &ash::Instance,
    physical_device: vk::PhysicalDevice,
    memory_type_bits: u32,
    context: &'static str,
) -> u32 {
    // SAFETY: `physical_device` belongs to `instance`, and the properties are
    // returned by value.
    let memory_properties =
        unsafe { instance.get_physical_device_memory_properties(physical_device) };
    (0..memory_properties.memory_type_count)
        .find(|index| memory_type_bits & (1 << index) != 0)
        .unwrap_or_else(|| {
            panic!(
                "{context}: the driver reports no memory type that can back a captured \
                 AHardwareBuffer"
            )
        })
}
