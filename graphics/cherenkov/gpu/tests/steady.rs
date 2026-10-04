//! Steady-state frames of unchanged content. Requires a working adapter;
//! CI uses Vulkan lavapipe.

use cherenkov::kurbo::Rect;
use cherenkov::{
    __engine_block as block, __engine_fn as split_fn, __engine_test as split_test,
    __engine_wait as wait,
};
use cherenkov::{Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{
    Gpu, GpuConfig,
    interop::{SharedDevice, wgpu},
};

split_fn! {
fn shared_device()
-> Result<(wgpu::Instance, wgpu::Adapter, wgpu::Device, wgpu::Queue), Box<dyn std::error::Error>> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        block!(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))?;
    let required_features = match adapter.get_info().backend {
        wgpu::Backend::Vulkan | wgpu::Backend::Metal => wgpu::Features::PASSTHROUGH_SHADERS,
        _ => wgpu::Features::empty(),
    };
    let (device, queue) = block!(adapter.request_device(&wgpu::DeviceDescriptor {
        required_features,
        ..wgpu::DeviceDescriptor::default()
    }))?;
    Ok((instance, adapter, device, queue))
}
}

/// Live device-memory allocations, sorted, as `(name, size)`.
fn allocations(device: &wgpu::Device) -> Option<Vec<(String, u64)>> {
    let report = device.generate_allocator_report()?;
    let mut live: Vec<(String, u64)> = report
        .allocations
        .into_iter()
        .map(|a| (a.name, a.size))
        .collect();
    live.sort();
    Some(live)
}

split_test! {
/// Re-recording unchanged content every frame keeps the device's
/// allocations fixed: the frame's uploads reuse the renderer's staging
/// memory instead of allocating per frame.
fn steady_frames_allocate_no_device_memory() -> Result<(), Box<dyn std::error::Error>> {
    let (instance, adapter, device, queue) = wait!(shared_device())?;
    let Some(before) = allocations(&device) else {
        eprintln!("skipping: backend exposes no allocator report");
        return Ok(());
    };
    let engine = wait!(Engine::<Gpu>::new(GpuConfig {
        device: Some(SharedDevice {
            instance,
            adapter,
            device: device.clone(),
            queue,
        }),
        ..GpuConfig::default()
    }))?;
    let surface = wait!(engine.surface(Offscreen::new((256, 256), OffscreenFormat::LinearF16)))?;
    let mut steady = None;
    for frame in 0..24 {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                for i in 0..4096u32 {
                    let (x, y) = (f64::from(i % 64) * 4.0, f64::from(i / 64) * 4.0);
                    c.fill(
                        Rect::new(x, y, x + 3.0, y + 3.0),
                        WorkingColor::new([1.0, 0.5, 0.25, 1.0]),
                    );
                }
            }));
        });
        wait!(engine.render(FrameTime::now()))?;
        let live = allocations(&device).ok_or("allocator report disappeared")?;
        if frame < 4 {
            continue;
        }
        match &steady {
            None => steady = Some(live),
            Some(steady) => assert_eq!(
                &live, steady,
                "frame {frame} changed the device allocations (before the engine: {before:?})"
            ),
        }
    }
    Ok(())
}
}
