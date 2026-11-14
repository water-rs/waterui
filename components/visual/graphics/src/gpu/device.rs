//! Opening a `wgpu` device on Android.
//!
//! Producers such as `waterkit-camera` deliver frames as `AHardwareBuffer`s
//! that import into the device through `wgpu_external_frame`'s
//! `ahardware_buffer` module. The import needs device extensions and a
//! feature `wgpu` never requests on its own —
//! `VK_ANDROID_external_memory_android_hardware_buffer`,
//! `VK_EXT_queue_family_foreign`, `VK_KHR_external_semaphore_fd` and
//! `samplerYcbcrConversion` — so they must be enabled when the device is
//! opened; the first import would be too late. [`request_device`] adds them
//! for the Vulkan adapters that support them and opens a plain device for
//! the rest.

use wgpu::{Adapter, Device, DeviceDescriptor, Queue};
use wgpu_external_frame::ahardware_buffer::{self, DeviceRequestError, DeviceRequirements};

/// Opens a device on `adapter` per `descriptor`, enabling the
/// `AHardwareBuffer`-import requirements a Vulkan adapter supports.
///
/// The decision is a capability check on the adapter, made before any
/// device exists — not a retry after a failed open. When
/// [`DeviceRequirements`] accepts the adapter,
/// [`ahardware_buffer::request_device`] opens the device with the import
/// extensions and the `samplerYcbcrConversion` feature inside wgpu-hal's
/// creation callback, keeping every feature and limit `descriptor` asks
/// for. An adapter that cannot import hardware buffers — a non-Vulkan
/// backend, or a Vulkan driver missing an extension or the feature — gets
/// the plain `Adapter::request_device` `descriptor` describes, and a later
/// `AHardwareBuffer` import on that device fails fast with
/// `wgpu_external_frame`'s own error.
///
/// The device also requests `TEXTURE_FORMAT_NV12` when the adapter offers
/// it: a driver that maps imported camera buffers to a Vulkan format aliases
/// them as NV12 textures, which importing producers such as `waterkit-camera`
/// require of the device.
///
/// # Errors
///
/// [`DeviceRequestError`] when `descriptor` asks for features or limits the
/// adapter lacks, or device creation fails.
pub async fn request_device(
    adapter: &Adapter,
    descriptor: &DeviceDescriptor<'_>,
) -> Result<(Device, Queue), DeviceRequestError> {
    match check_import_requirements(adapter) {
        Ok(()) => {
            tracing::info!("opening the wgpu device with the AHardwareBuffer import requirements");
            let mut descriptor = descriptor.clone();
            descriptor.required_features |=
                adapter.features() & wgpu::Features::TEXTURE_FORMAT_NV12;
            ahardware_buffer::request_device(adapter, &descriptor)
        }
        Err(reason) => {
            tracing::info!(
                %reason,
                "the adapter cannot import AHardwareBuffers; opening a plain wgpu device"
            );
            adapter
                .request_device(descriptor)
                .await
                .map_err(DeviceRequestError::from)
        }
    }
}

/// Whether `adapter` can open a device carrying the `AHardwareBuffer`
/// import requirements; `Err` names the reason it cannot, for the log.
fn check_import_requirements(adapter: &Adapter) -> Result<(), String> {
    let backend = adapter.get_info().backend;
    if backend != wgpu::Backend::Vulkan {
        return Err(format!("the adapter's backend is {backend:?}, not Vulkan"));
    }
    // SAFETY: `adapter` reports Vulkan, so `as_hal` exposes its
    // `wgpu::hal::vulkan::Adapter`; the guard only reads capabilities and is
    // dropped before the device is opened.
    let hal_adapter = unsafe { adapter.as_hal::<wgpu::hal::api::Vulkan>() }
        .expect("a Vulkan adapter exposes its hal adapter");
    DeviceRequirements::new(&hal_adapter)
        .map(|_| ())
        .map_err(|reason| reason.to_string())
}
