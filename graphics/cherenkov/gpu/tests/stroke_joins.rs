//! Stroke joins and caps without an analytic form: a bevelled rect stroke
//! matches its stroked-to-path form, a miter limit below √2 bevels, a limit
//! at √2 stays sharp, unequal line caps render, and a bevelled
//! continuous-rect stroke draws.

use cherenkov::kurbo::{Line, Rect, Shape as _};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{ContinuousRect, Draw, WorkingColor};
use cherenkov::{Engine, EngineError, Offscreen, OffscreenFormat};
use cherenkov_gpu::{Gpu, GpuConfig};

const CLEAR: WorkingColor = WorkingColor::new([0.0, 0.0, 0.0, 1.0]);
const RED: WorkingColor = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);

const RECT: Rect = Rect::new(16.0, 16.0, 48.0, 48.0);

split_fn! {
fn engine() -> Result<Option<Engine<Gpu>>, Box<dyn std::error::Error>> {
    match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => Ok(Some(engine)),
        Err(EngineError::Backend(_)) => Ok(None),
        Err(e) => Err(e.into()),
    }
}
}

split_fn! {
// One engine per test, reused across renders: each call builds its own
// surface, so the scenes stay independent while construction is shared.
fn render(
    engine: &Engine<Gpu>,
    draw: impl FnOnce(&mut cherenkov::Recorder),
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.clear_color(CLEAR);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    surface.update(|tx| {
        tx[&layer].content(surface.record(draw));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

fn px(readback: &cherenkov::Readback, x: u32, y: u32) -> [f32; 4] {
    readback.pixels[(y * readback.width + x) as usize]
}

fn stroke(style: kurbo::Stroke) -> impl FnOnce(&mut cherenkov::Recorder) {
    move |c| {
        c.stroke(RECT, style, RED);
    }
}

const fn style(join: kurbo::Join, miter_limit: f64) -> kurbo::Stroke {
    kurbo::Stroke::new(8.0)
        .with_join(join)
        .with_miter_limit(miter_limit)
}

split_test! {
fn a_bevel_rect_stroke_matches_its_path_form() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let analytic = wait!(render(&engine, stroke(style(kurbo::Join::Bevel, 4.0))))?;
    let path_form = wait!(render(&engine, |c| {
        c.stroke(RECT.to_path(0.1), style(kurbo::Join::Bevel, 4.0), RED);
    }))?;
    assert_eq!(
        analytic.pixels, path_form.pixels,
        "a bevelled rect stroke is rasterized from its stroked path"
    );
    Ok(())
}
}

split_test! {
fn a_miter_limit_below_root_two_bevels() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let mitered = wait!(render(&engine, stroke(style(kurbo::Join::Miter, 1.0))))?;
    let bevelled = wait!(render(&engine, stroke(style(kurbo::Join::Bevel, 4.0))))?;
    assert_eq!(
        mitered.pixels, bevelled.pixels,
        "a miter limit of 1.0 cannot hold a right angle, so it bevels"
    );
    Ok(())
}
}

split_test! {
fn a_miter_limit_at_root_two_stays_analytic() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let at_limit = wait!(render(
        &engine,
        stroke(style(kurbo::Join::Miter, std::f64::consts::SQRT_2)),
    ))?;
    let high_limit = wait!(render(&engine, stroke(style(kurbo::Join::Miter, 4.0))))?;
    assert_eq!(
        at_limit.pixels, high_limit.pixels,
        "a right angle's miter ratio is √2, so the limit still holds it"
    );
    // Inside the outer sharp corner the miter is drawn; a bevel cuts it.
    let [r, ..] = px(&at_limit, 13, 13);
    assert!(r > 0.5, "miter corner: {r}");
    let bevelled = wait!(render(&engine, stroke(style(kurbo::Join::Bevel, 4.0))))?;
    let [r, ..] = px(&bevelled, 13, 13);
    assert!(r < 0.05, "bevel corner: {r}");
    Ok(())
}
}

split_test! {
fn a_line_with_unequal_caps_renders() -> Result<(), Box<dyn std::error::Error>> {
    let style = kurbo::Stroke::new(8.0)
        .with_start_cap(kurbo::Cap::Butt)
        .with_end_cap(kurbo::Cap::Square);
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let readback = wait!(render(&engine, |c| {
        c.stroke(Line::new((16.0, 32.0), (48.0, 32.0)), style, RED);
    }))?;
    // The square end extends half a width past the line's end.
    let [r, ..] = px(&readback, 50, 32);
    assert!(r > 0.5, "square end: {r}");
    // The butt end stops at the line's start.
    let [r, ..] = px(&readback, 14, 32);
    assert!(r < 0.05, "butt end: {r}");
    Ok(())
}
}

split_test! {
fn a_bevel_continuous_rect_stroke_renders() -> Result<(), Box<dyn std::error::Error>> {
    let continuous = ContinuousRect::new(RECT, 0.0).with_smoothing(0.6);
    let Some(engine) = wait!(engine())? else {
        return Ok(());
    };
    let readback = wait!(render(&engine, move |c| {
        c.stroke(continuous, style(kurbo::Join::Bevel, 4.0), RED);
    }))?;
    let [r, ..] = px(&readback, 32, 16);
    assert!(r > 0.5, "edge midpoint: {r}");
    Ok(())
}
}
