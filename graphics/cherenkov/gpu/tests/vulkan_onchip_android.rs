//! Pixel-side verification that the engine's Vulkan device is opened
//! with the capabilities on-chip composition needs (issue #2191). Build
//! for aarch64-linux-android and run the artifact on a Pixel 9 Pro over
//! adb:
//!
//! ```text
//! cargo ndk --target aarch64-linux-android --platform 34 \
//!     build -p cherenkov-gpu --test vulkan_onchip_android
//! BIN=$(ls -t target/aarch64-linux-android/debug/deps/vulkan_onchip_android-* | head -1)
//! adb push "$BIN" /data/local/tmp/vulkan_onchip_android
//! adb shell 'cd /data/local/tmp && ./vulkan_onchip_android --nocapture'
//! ```
//!
//! Per-pixel ordered attachment access is a required result, not an
//! optional one: on-chip programmable blending is the engine's hardware
//! floor, and every device of the floor — Mali-G715 here — publishes
//! either `VK_EXT_rasterization_order_attachment_access` or its ARM
//! original alongside `VK_KHR_dynamic_rendering_local_read`. A device
//! that reports less composes every intermediate through memory, which
//! is the regression this test exists to catch.
#![cfg(target_os = "android")]

use std::ffi::CStr;

use cherenkov_gpu::{
    GpuConfig,
    interop::{SharedDevice, wgpu},
};

/// The device the engine opens for itself, and what it enabled.
#[test]
fn the_engine_device_can_read_attachments_per_pixel() {
    let shared = SharedDevice::create(&GpuConfig::default()).expect("shared device");
    assert_eq!(
        shared.adapter.get_info().backend,
        wgpu::Backend::Vulkan,
        "the Pixel test must run on Vulkan"
    );
    // SAFETY: the hal view borrows `shared.device` for this scope and
    // only its enabled-extension list is read.
    let hal = unsafe { shared.device.as_hal::<wgpu::hal::vulkan::Api>() }
        .expect("a Vulkan adapter opens a Vulkan device");
    let enabled = hal.enabled_device_extensions();
    eprintln!("enabled device extensions:");
    for name in enabled {
        eprintln!("  {}", name.to_string_lossy());
    }
    let has = |name: &CStr| enabled.contains(&name);
    assert!(
        has(ash::khr::dynamic_rendering_local_read::NAME),
        "VK_KHR_dynamic_rendering_local_read is how a draw reads a slot \
         of the current rendering scope"
    );
    assert!(
        has(ash::vk::EXT_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_NAME)
            || has(ash::vk::ARM_RASTERIZATION_ORDER_ATTACHMENT_ACCESS_NAME),
        "ordered colour attachment access is what makes those reads \
         see what earlier fragments of the same pixel wrote"
    );
}
