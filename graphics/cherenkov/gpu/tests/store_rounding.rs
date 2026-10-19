//! `Rgba16Float` stores round to the nearest f16, not toward zero (#2773).
//!
//! Apple GPUs store float pixels toward zero unless the shader is compiled
//! with `-ftexture-write-rounding-mode=rte`; `build.rs` compiles the
//! engine's precompiled Metal modules that way. An opaque fill whose
//! working-space value is off the f16 grid must read back as the nearest
//! f16, which differs from the toward-zero neighbour for every channel
//! below.
#![cfg(target_vendor = "apple")]

use cherenkov::kurbo::Rect;
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{Draw, Engine, FrameTime, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{Gpu, GpuConfig};

/// The f16 spacing in `[0.5, 1)`.
const STEP: f32 = 1.0 / 2048.0;

split_test! {
fn opaque_fill_stores_nearest_f16() -> Result<(), Box<dyn std::error::Error>> {
    // Each channel sits more than half a step above an f16 value in
    // [0.5, 1), so the nearest f16 is the one above, and toward zero is the
    // one below.
    let color = [0.787_412_2_f32, 0.6, 0.95];
    for c in color {
        let steps = c / STEP;
        assert!(steps.fract() > 0.55 && steps.fract() < 0.95, "{c} is not a clear round-up case");
    }
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {}))?;
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
        tx[&layer].content(surface.record(|r| {
            r.fill(
                Rect::new(0.0, 0.0, 8.0, 8.0),
                WorkingColor::new([color[0], color[1], color[2], 1.0]),
            );
        }));
    });
    wait!(engine.render(FrameTime::now()))?;
    let pixels = wait!(surface.readback())?.pixels;
    let nearest = color.map(|c| (c / STEP).round() * STEP);
    for (i, px) in pixels.iter().enumerate() {
        assert_eq!(
            [px[0], px[1], px[2]],
            nearest,
            "pixel {i} is not the nearest f16 of {color:?}"
        );
    }
    Ok(())
}
}
