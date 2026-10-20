//! Diagnostic for #2726, never merged: where a GPU test's time goes on this
//! adapter. Writes one line per phase to `$PROBE_OUT`.

use std::fmt::Write as _;
use std::time::Instant;

use cherenkov::kurbo::Rect;
use cherenkov::{
    __engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait,
};
use cherenkov::{Draw, Engine, Offscreen, OffscreenFormat, WorkingColor};
use cherenkov_gpu::{Gpu, GpuConfig};

split_fn! {
fn card(engine: &Engine<Gpu>, alpha: f32) -> Result<(), Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16), || {}))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            let card = Rect::new(24., 24., 104., 104.);
            c.shadow(
                card,
                cherenkov::Shadow::new(6.0, WorkingColor::new([0., 0., 0., 1.])).offset((2., 3.)),
            );
            c.fill(card, WorkingColor::new([0.9, 0.3, 0.1, alpha]));
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let _ = wait!(surface.readback())?;
    Ok(())
}
}

split_test! {
fn phases() -> Result<(), Box<dyn std::error::Error>> {
    let mut out = String::new();
    let start = Instant::now();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    writeln!(out, "engine_new_1 {:?}", start.elapsed())?;
    let t = Instant::now();
    wait!(card(&engine, 1.0))?;
    writeln!(out, "first_card {:?}", t.elapsed())?;
    let t = Instant::now();
    wait!(card(&engine, 0.5))?;
    writeln!(out, "second_card {:?}", t.elapsed())?;
    drop(engine);
    let t = Instant::now();
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    writeln!(out, "engine_new_2 {:?}", t.elapsed())?;
    let t = Instant::now();
    wait!(card(&engine, 1.0))?;
    writeln!(out, "first_card_engine_2 {:?}", t.elapsed())?;
    writeln!(out, "total {:?}", start.elapsed())?;
    std::fs::write(std::env::var("PROBE_OUT")?, out)?;
    Ok(())
}
}
