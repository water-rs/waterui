//! A translucent fill in a child layer composites exactly once over the
//! clear colour.

use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{Draw, WorkingColor};
use cherenkov::{Engine, EngineError, Offscreen, OffscreenFormat};
use cherenkov_gpu::{Gpu, GpuConfig};

const GREY: WorkingColor = WorkingColor::new([0.8, 0.8, 0.8, 1.0]);
const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
const BLUE: WorkingColor = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);

split_test! {
fn a_disjoint_opacity_layer_passes_through() -> Result<(), Box<dyn std::error::Error>> {
    let engine = match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.clear_color(GREY);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].opacity(0.5f32);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(Rect::new(4., 4., 28., 28.), RED);
            c.fill(Rect::new(36., 36., 60., 60.), BLUE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let stats = engine.stats();
    assert_eq!(stats.passes, 1, "disjoint rects should not isolate");
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    // 0.5-opacity red over 0.8 grey: (0.9, 0.4, 0.4, 1).
    let [r, g, b, a] = px(16, 16);
    assert!(
        (r - 0.9).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.4).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "red: {r} {g} {b} {a}"
    );
    let [r, g, b, a] = px(48, 48);
    assert!(
        (r - 0.4).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.9).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "blue: {r} {g} {b} {a}"
    );
    Ok(())
}
}

split_test! {
fn an_overlapping_opacity_layer_still_isolates() -> Result<(), Box<dyn std::error::Error>> {
    let engine = match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.clear_color(GREY);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].opacity(0.5f32);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(Rect::new(8., 8., 40., 40.), RED);
            c.fill(Rect::new(24., 24., 56., 56.), BLUE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let stats = engine.stats();
    assert_eq!(
        stats.passes, if cfg!(target_vendor = "apple") { 2 } else { 3 },
        "overlapping rects isolate in a fused tile epoch on Metal"
    );
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    // In the group, blue-over-red blends opaquely (0, 0, 1); then the whole
    // group composites at 0.5 over grey: (0.4, 0.4, 0.9).
    let [r, g, b, a] = px(32, 32);
    assert!(
        (r - 0.4).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.9).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "overlap: {r} {g} {b} {a}"
    );
    // Red-only region: 0.5 red over grey → (0.9, 0.4, 0.4).
    let [r, g, b, a] = px(16, 16);
    assert!(
        (r - 0.9).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.4).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "red: {r} {g} {b} {a}"
    );
    Ok(())
}
}

split_test! {
fn isolation_uses_tile_storage_or_a_region_sized_scratch() -> Result<(), Box<dyn std::error::Error>> {
    let config = GpuConfig {
        timestamps: true,
        ..GpuConfig::default()
    };
    let engine = match wait!(Engine::<Gpu>::new(config)) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((256, 256), OffscreenFormat::LinearF16)))?;
    surface.clear_color(GREY);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].opacity(0.5f32);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            // Two overlapping rects in the bottom-right corner.
            c.fill(Rect::new(160., 160., 224., 224.), RED);
            c.fill(Rect::new(192., 192., 250., 250.), BLUE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let stats = engine.stats();
    assert_eq!(stats.passes, if cfg!(target_vendor = "apple") { 2 } else { 3 });
    let phases = stats.phases;
    assert!(phases.lower_seconds >= 0.0 && phases.stamp_seconds >= 0.0);
    assert!(
        phases.lower_seconds + phases.encode_seconds + phases.wait_seconds > 0.0,
        "a non-empty frame must measure some render-thread CPU time: {phases:?}"
    );
    // Per-pass regions come with the frame's timing; an adapter without
    // TIMESTAMP_QUERY reports none, and only the pixels are checked.
    if let Some(timing) = wait!(engine.finish_timings())?.pop() {
        if cfg!(target_vendor = "apple") {
            assert_eq!(timing.passes.len(), 2);
            assert_eq!(timing.passes[1].name, "tile composition");
            assert_eq!((timing.passes[1].width, timing.passes[1].height), (256, 256));
        } else {
        let scratch = timing
            .passes
            .iter()
            .find(|p| p.name.starts_with("scratch"))
            .expect("a scratch pass");
        assert!(
            scratch.width < 256 && scratch.height < 256,
            "scratch region {}x{} should be tight",
            scratch.width,
            scratch.height
        );
        assert!(
            scratch.width <= 100 && scratch.height <= 100,
            "{}x{}",
            scratch.width,
            scratch.height
        );
        }
    }
    let rb = wait!(surface.readback())?;
    let px = |x: u32, y: u32| rb.pixels[(y * rb.width + x) as usize];
    let [r, g, b, a] = px(208, 208);
    assert!(
        (r - 0.4).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.9).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "overlap: {r} {g} {b} {a}"
    );
    let [r, g, b, a] = px(170, 170);
    assert!(
        (r - 0.9).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.4).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "red: {r} {g} {b} {a}"
    );
    Ok(())
}
}

split_test! {
fn a_child_layer_draws_once() -> Result<(), Box<dyn std::error::Error>> {
    let engine = match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.clear_color(WorkingColor::new([0.8, 0.8, 0.8, 1.0]));
    let layer = surface.layer();
    surface.update(|tx| {
        tx[&layer].transform(Affine::IDENTITY).opacity(1.0f32);
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| {
            c.fill(
                Rect::new(8., 8., 56., 56.),
                WorkingColor::new([0.0, 0.0, 1.0, 0.5]),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    let [r, g, b, a] = readback.pixels[(32 * readback.width + 32) as usize];
    assert!(
        (r - 0.4).abs() < 2e-3
            && (g - 0.4).abs() < 2e-3
            && (b - 0.9).abs() < 2e-3
            && (a - 1.0).abs() < 1e-6,
        "centre: {r} {g} {b} {a}"
    );
    Ok(())
}
}
