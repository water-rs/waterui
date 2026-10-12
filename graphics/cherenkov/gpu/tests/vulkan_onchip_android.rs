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

use cherenkov::kurbo::Rect;
use cherenkov::{Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{
    Gpu, GpuConfig,
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

/// A layer whose own contents overlap has to be composed in isolation
/// before its opacity applies, so this scene is exactly one tile epoch:
/// the isolation intermediate is written and read inside one rendering
/// scope. The pixels say the epoch produced the right image, and the
/// engine's memory report says what the attachments cost — the driver's
/// own commitment figure, which is the whole point of keeping them in
/// tile memory (#2191).
#[test]
fn an_isolated_layer_composes_on_chip_and_reports_its_attachments() {
    const GREY: WorkingColor = WorkingColor::new([0.8, 0.8, 0.8, 1.0]);
    const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
    const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);

    let engine = Engine::<Gpu>::new(GpuConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {})
        .expect("offscreen surface");
    surface.clear_color(GREY);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].opacity(0.5f32);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(Rect::new(4., 4., 40., 40.), RED);
            c.fill(Rect::new(24., 24., 60., 60.), BLUE);
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let rb = surface.readback().expect("readback");
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    let close = |actual: [f32; 4], expected: [f32; 4]| {
        assert!(
            actual
                .iter()
                .zip(expected)
                .all(|(a, e)| (a - e).abs() < 2e-3),
            "pixel {actual:?}, expected {expected:?}"
        );
    };
    // Red alone at half opacity over the grey clear.
    close(px(12, 12), [0.9, 0.4, 0.4, 1.0]);
    // Where the layer's own rects overlap, blue wins inside the layer
    // and the layer's opacity applies once to the result — the whole
    // reason the layer is isolated.
    close(px(32, 32), [0.4, 0.4, 0.9, 1.0]);
    // Blue alone.
    close(px(52, 52), [0.4, 0.4, 0.9, 1.0]);
    // Outside the layer the clear colour stands.
    close(px(2, 2), [0.8, 0.8, 0.8, 1.0]);

    let memory = engine.memory();
    eprintln!(
        "tile attachments: reserved {:?}, committed {:?}",
        memory.tile_reserved, memory.tile_committed
    );
    assert!(
        memory.tile_reserved.0 > 0,
        "the epoch's transient attachments are allocated and reported"
    );
    let committed = memory
        .tile_committed
        .expect("Vulkan reports commitment for lazily allocated memory");
    assert!(
        committed.0 <= memory.tile_reserved.0,
        "committed {committed:?} exceeds the reserved {:?} it is part of",
        memory.tile_reserved
    );
}
