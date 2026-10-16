//! General silhouette convolution.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::{__engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Draw, Engine, EvenOdd, FrameTime, Offscreen, OffscreenFormat, Shadow, WorkingColor,
};
use kurbo::BezPath;
#[path = "../../tests/common/silhouette_shadows.rs"]
mod common;
split_test! {
fn live_shadows_keep_offscreen_contributors_and_outer_clips() {
    wait!(common::retained_and_padded::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default()));
}
}

split_test! {
fn invalid_silhouette_inputs_fail_explicitly() {
    wait!(common::invalid::<cherenkov_cpu::Raster>(cherenkov_cpu::RasterConfig::default));
}
}

/// The reproduction's contour — `(0.25, 0.25) → (0.75, 0.25) →
/// (0.75, 0.75) → (0.25, 0.75)` — scaled to the 64×64 surfaces every
/// scene in this file uses, drawn `times` times in one path. The ×63
/// scale keeps the contour edges on quarter-pixels: boundary pixels are
/// covered a quarter by each contour, the issue's case that a doubled
/// winding over-covers.
fn caster(times: usize) -> BezPath {
    let mut p = BezPath::new();
    for _ in 0..times {
        p.move_to((0.25 * 63.0, 0.25 * 63.0));
        p.line_to((0.75 * 63.0, 0.25 * 63.0));
        p.line_to((0.75 * 63.0, 0.75 * 63.0));
        p.line_to((0.25 * 63.0, 0.75 * 63.0));
        p.close_path();
    }
    p
}

split_test! {
/// The caster's winding is resolved before pixel area is integrated: a
/// contour drawn twice is identical to once under `NonZero` and to no
/// shadow under `EvenOdd`.
fn silhouette_resolves_winding_before_integrating_pixel_area() {
    let engine = wait!(Engine::<cherenkov_cpu::Raster>::new(
        cherenkov_cpu::RasterConfig::default()
    ))
    .expect("backend");
    let doubled = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))
        .expect("doubled");
    let reference = wait!(engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16), || {}))
        .expect("reference");
    let shadow = Shadow::new(3.0, WorkingColor::BLACK);
    doubled.update(|tx| {
        tx[doubled.root()].content(doubled.record(|r| {
            r.shadow(caster(2), shadow);
        }));
    });
    reference.update(|tx| {
        tx[reference.root()].content(reference.record(|r| {
            r.shadow(caster(1), shadow);
        }));
    });
    wait!(engine.render(FrameTime::now())).expect("render");
    // Both scenes lower through the same code path, so the comparison is
    // exact rather than within this file's 0.001 tolerance.
    assert_eq!(
        wait!(doubled.readback()).expect("doubled pixels").pixels,
        wait!(reference.readback())
            .expect("reference pixels")
            .pixels,
        "a doubled contour casts the same shadow as a single one under NonZero"
    );
    doubled.update(|tx| {
        tx[doubled.root()].content(doubled.record(|r| {
            r.shadow(EvenOdd(caster(2)), shadow);
        }));
    });
    reference.update(|tx| {
        tx[reference.root()].content(reference.record(|_| {}));
    });
    wait!(engine.render(FrameTime::now())).expect("render");
    assert_eq!(
        wait!(doubled.readback()).expect("doubled pixels").pixels,
        wait!(reference.readback())
            .expect("reference pixels")
            .pixels,
        "the doubled region is outside under EvenOdd and casts no shadow"
    );
}
}
