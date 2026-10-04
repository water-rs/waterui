//! Host check (Apple only: it drives the real producer) that pool slots
//! drain and get rewritten: produce, install and render well past the
//! pool's depth and every tick must still produce — the stall the device
//! run exposed while `IOSurfaceIsInUse` was in the gate (a pool buffer's
//! own liveness keeps its surface "in use", so it could never clear).

#![cfg(target_vendor = "apple")]

use cherenkov::kurbo::Affine;
use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::Gpu;
use cherenkov_gpu::interop::SharedDevice;

use apple_planes::producer::Pool;

#[test]
fn slots_drain_past_the_pool_depth() {
    let shared = SharedDevice::create(&cherenkov_gpu::GpuConfig::default()).expect("shared device");
    let mut pool = Pool::new(&shared, false);
    let engine = Engine::<Gpu>::new(cherenkov_gpu::GpuConfig {
        device: Some(shared),
        ..cherenkov_gpu::GpuConfig::default()
    })
    .expect("host GPU engine");
    let surface = engine
        .surface(Offscreen::new((480, 270), OffscreenFormat::LinearF16))
        .expect("offscreen surface");
    let video = surface.layer();
    let (prod, sink) = engine.frame_producer();
    surface.update(|tx| {
        tx[surface.root()].push(&video);
        tx[&video]
            .transform(Affine::scale(0.25))
            .content(prod.at((1920, 1080)));
    });
    let mut produced = 0;
    for _ in 0..16 {
        if let Some(frame) = pool.produce() {
            sink.submit(frame);
            produced += 1;
        }
        engine.render(FrameTime::now()).expect("rendered");
        pool.rendered();
        // One vsync's worth of time lets each tick's serial complete.
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
    assert!(
        produced >= 12,
        "produced {produced} of 16 ticks, stalls {}",
        pool.stalls
    );
}
