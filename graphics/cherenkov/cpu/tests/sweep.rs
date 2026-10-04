//! Sweep domain normalization must terminate and preserve continuation.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{
    Draw, Engine, Extend, FrameTime, Offscreen, OffscreenFormat, Paint, SweepGradient, WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};

#[test]
fn sweep_wraps_negative_spans_and_applies_paint_coordinates() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((32, 32), OffscreenFormat::LinearF32))
        .expect("surface");
    let gradient = SweepGradient::new((8.5, 16.5), 0.0, -std::f64::consts::PI)
        .stop(0.0, WorkingColor::new([1.0, 0.0, 0.0, 1.0]))
        .stop(1.0, WorkingColor::new([0.0, 0.0, 1.0, 1.0]))
        .extend(Extend::None);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 32.0, 32.0),
                Paint::from(gradient).transformed(Affine::translate((8.0, 0.0))),
            );
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let pixels = surface.readback().expect("pixels").pixels;
    let bottom = pixels[24 * 32 + 16];
    assert!((bottom[0] - 0.5).abs() < 1e-5 && (bottom[2] - 0.5).abs() < 1e-5);
    assert_eq!(pixels[8 * 32 + 16][3].to_bits(), 0.0_f32.to_bits());
}

#[test]
fn nonfinite_sweep_angles_are_errors() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let surface = engine
        .surface(Offscreen::new((4, 4), OffscreenFormat::LinearF32))
        .expect("surface");
    for end in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.fill(
                    Rect::new(0.0, 0.0, 4.0, 4.0),
                    SweepGradient::new((2.0, 2.0), 0.0, end),
                );
            }));
        });
        assert!(engine.render(FrameTime::now()).is_err());
    }
}
