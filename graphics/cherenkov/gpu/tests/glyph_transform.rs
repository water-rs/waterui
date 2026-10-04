//! Per-glyph transforms (#69): translation folding, outline coverage and
//! validation errors.

use cherenkov::kurbo::{Affine, Stroke};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Draw, Engine, FontSource, FrameTime, Glyph, GlyphRun, GlyphStyle, Offscreen, OffscreenFormat,
    RenderError, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig};
use nami::Binding;

split_fn! {
fn engine_and_font() -> (Engine<Gpu>, cherenkov::Font) {
    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default())).expect("GPU engine");
    let font = engine
        .font(FontSource::bytes(
            std::fs::read("../scenes/fonts/NotoSans.ttf").expect("font"),
        ))
        .expect("registered font");
    (engine, font)
}
}

const fn glyph(x: f32, y: f32, transform: Option<Affine>) -> Glyph {
    Glyph {
        id: 36,
        x,
        y,
        transform,
    }
}

fn run(font: cherenkov::FontId, glyphs: Vec<Glyph>, style: GlyphStyle) -> GlyphRun {
    GlyphRun {
        font,
        size: 38.0,
        coords: Vec::new().into(),
        glyphs: glyphs.into(),
        style,
    }
}

split_fn! {
fn surface_with(
    engine: &Engine<Gpu>,
    run: GlyphRun,
) -> (cherenkov::Surface<Gpu>, Binding<GlyphRun>) {
    let surface = wait!(engine
        .surface(Offscreen::new((96, 72), OffscreenFormat::LinearF16)))
        .expect("surface");
    let value = Binding::container(run);
    let content = surface.record(|c| {
        c.glyphs(value.clone(), WorkingColor::WHITE);
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    (surface, value)
}
}

split_fn! {
fn pixels(surface: &cherenkov::Surface<Gpu>) -> Vec<[u32; 4]> {
    wait!(surface
        .readback())
        .expect("pixels")
        .pixels
        .iter()
        .map(|p| p.map(f32::to_bits))
        .collect()
}
}

split_test! {
fn pure_translation_folds_into_position() {
    let (engine, font) = wait!(engine_and_font());
    for style in [GlyphStyle::Fill, GlyphStyle::Stroke(Stroke::new(1.0))] {
        let moved = run(
            font.id(),
            vec![
                glyph(12.25, 52.5, Some(Affine::translate((3.0, -4.0)))),
                glyph(45.5, 52.5, None),
            ],
            style.clone(),
        );
        let direct = run(
            font.id(),
            vec![glyph(15.25, 48.5, None), glyph(45.5, 52.5, None)],
            style,
        );
        let (surface, _) = wait!(surface_with(&engine, moved));
        wait!(engine.render(FrameTime::now())).expect("translated render");
        let a = wait!(pixels(&surface));
        let (surface, _) = wait!(surface_with(&engine, direct));
        wait!(engine.render(FrameTime::now())).expect("direct render");
        assert_eq!(a, wait!(pixels(&surface)), "folded translation must be identical");
    }
}
}

split_test! {
fn rotated_glyph_renders_outline_coverage() {
    let (engine, font) = wait!(engine_and_font());
    let mut transformed = run(
        font.id(),
        vec![
            glyph(12.25, 52.5, None),
            glyph(45.5, 52.5, Some(Affine::rotate(0.6))),
        ],
        GlyphStyle::Fill,
    );
    let plain = run(
        font.id(),
        vec![glyph(12.25, 52.5, None), glyph(45.5, 52.5, None)],
        GlyphStyle::Fill,
    );
    let (surface, value) = wait!(surface_with(&engine, plain));
    wait!(engine.render(FrameTime::now())).expect("plain render");
    let plain_pixels = wait!(pixels(&surface));

    value.set(transformed.clone());
    wait!(engine.render(FrameTime::now())).expect("rotated render");
    let rotated_pixels = wait!(pixels(&surface));
    assert_ne!(
        plain_pixels, rotated_pixels,
        "a rotated glyph must change coverage"
    );

    // Back to `None`: the op count shrinks, forcing a rebuild that must
    // reproduce the untransformed frame exactly.
    let mut glyphs = transformed.glyphs.to_vec();
    glyphs[1].transform = None;
    transformed.glyphs = glyphs.into();
    value.set(transformed);
    wait!(engine.render(FrameTime::now())).expect("restored render");
    assert_eq!(plain_pixels, wait!(pixels(&surface)));
}
}

split_test! {
fn invalid_transform_is_an_error() {
    let (engine, font) = wait!(engine_and_font());
    for transform in [
        Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
        Affine::scale(0.0),
    ] {
        let run = run(
            font.id(),
            vec![glyph(12.25, 52.5, Some(transform))],
            GlyphStyle::Fill,
        );
        let (surface, _) = wait!(surface_with(&engine, run));
        match wait!(engine.render(FrameTime::now())) {
            Err(RenderError::Render(_)) => {}
            other => panic!("expected Render error, got {other:?}"),
        }
        drop(surface);
    }
}
}
