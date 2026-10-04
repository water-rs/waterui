//! Host check (Apple only: it drives the real producer) that the
//! engine-composited path decodes the producer's output: `Pool`
//! fills an `IOSurface`-backed buffer, wraps it as the frame the
//! harness installs, and an offscreen surface's readback is sampled at
//! each quadrant centre.
//!
//! The regression this guards is the `IOSurface` → Metal texture →
//! wgpu import chain — a wrong plane, a wrong format declaration or a
//! byte order swap would show as wrong quadrant colours, the defect
//! class a device run otherwise reports only through the heartbeat.

#![cfg(target_vendor = "apple")]

use cherenkov::kurbo::Affine;
use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_gpu::Gpu;
use cherenkov_gpu::interop::SharedDevice;

use apple_planes::producer::Pool;

/// A pixel's hue as dominance: which channel leads, or `None` when all
/// three are equal (the gray quadrant).
fn hue([r, g, b]: [f32; 3]) -> Option<usize> {
    let max = r.max(g).max(b);
    if max - r.min(g).min(b) < 0.02 {
        return None;
    }
    [r, g, b].iter().position(|&c| c >= max)
}

#[test]
fn the_engine_decodes_the_produced_frame() {
    let shared = SharedDevice::create(&cherenkov_gpu::GpuConfig::default()).expect("shared device");
    let mut pool = Pool::new(&shared, false);
    let frame = pool.produce().expect("a produced frame");

    let engine = Engine::<Gpu>::new(cherenkov_gpu::GpuConfig {
        device: Some(shared),
        ..cherenkov_gpu::GpuConfig::default()
    })
    .expect("host GPU engine");
    let (sw, sh) = (480u32, 270u32);
    let surface = engine
        .surface(Offscreen::new((sw, sh), OffscreenFormat::LinearF16))
        .expect("offscreen surface");
    let video = surface.layer();
    let (prod, sink) = engine.frame_producer();
    sink.submit(frame);
    surface.update(|tx| {
        tx[surface.root()].push(&video);
        // The full frame covers the surface: 1920×1080 scaled by 0.25.
        tx[&video].transform(Affine::scale(0.25));
        tx[&video].content(prod.at((1920, 1080)));
    });
    engine.render(FrameTime::now()).expect("frame renders");
    let read = surface.readback().expect("readable surface");
    assert_eq!((read.width, read.height), (sw, sh));

    let pixel = |x: u32, y: u32| read.pixels[(y * sw + x) as usize];
    // Quadrant centres in the displayed frame: red, green, blue, gray.
    let quadrants = [
        pixel(sw / 4, sh / 4),
        pixel(sw * 3 / 4, sh / 4),
        pixel(sw / 4, sh * 3 / 4 - sh / 16),
        pixel(sw * 3 / 4, sh * 3 / 4 - sh / 16),
    ];
    for (at, [_, _, _, a]) in quadrants.iter().copied().enumerate() {
        assert!(a > 0.99, "quadrant {at} is not opaque: {quadrants:?}");
    }
    let hues: Vec<Option<usize>> = quadrants
        .iter()
        .map(|&[r, g, b, _]| hue([r, g, b]))
        .collect();
    assert_eq!(
        hues,
        [Some(0), Some(1), Some(2), None],
        "quadrant hues: {quadrants:?}"
    );
}
