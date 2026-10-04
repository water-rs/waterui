//! Per-glyph transforms (#69): translation folding, outline coverage and
//! validation errors.

#![cfg(not(target_arch = "wasm32"))]
use cherenkov::kurbo::{Affine, Stroke};
use cherenkov::{
    Draw, Engine, FontSource, FrameTime, Glyph, GlyphRun, GlyphStyle, Offscreen, OffscreenFormat,
    RenderError, WorkingColor,
};
use cherenkov_cpu::{Raster as Gpu, RasterConfig as GpuConfig};
use nami::Binding;

const COLR_FONT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scenes/corpus/text-colr/resources/",
    "04dd974b8bf7e440fd8927b6af23764194287cee28d15acda268c5e0a51091f5"
);

fn engine_and_font() -> (Engine<Gpu>, cherenkov::Font) {
    let engine = Engine::<Gpu>::new(GpuConfig::default()).expect("CPU engine");
    let font = engine
        .font(FontSource::bytes(
            std::fs::read("../scenes/fonts/NotoSans.ttf").expect("font"),
        ))
        .expect("registered font");
    (engine, font)
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

fn surface_with(
    engine: &Engine<Gpu>,
    run: GlyphRun,
) -> (cherenkov::Surface<Gpu>, Binding<GlyphRun>) {
    let surface = engine
        .surface(Offscreen::new((96, 72), OffscreenFormat::LinearF16))
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

fn pixels(surface: &cherenkov::Surface<Gpu>) -> Vec<[u32; 4]> {
    surface
        .readback()
        .expect("pixels")
        .pixels
        .iter()
        .map(|p| p.map(f32::to_bits))
        .collect()
}

#[test]
fn pure_translation_folds_into_position() {
    let (engine, font) = engine_and_font();
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
        let (surface, _) = surface_with(&engine, moved);
        engine.render(FrameTime::now()).expect("translated render");
        let a = pixels(&surface);
        let (surface, _) = surface_with(&engine, direct);
        engine.render(FrameTime::now()).expect("direct render");
        assert_eq!(a, pixels(&surface), "folded translation must be identical");
    }
}

#[test]
fn rotated_glyph_renders_outline_coverage() {
    let (engine, font) = engine_and_font();
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
    let (surface, value) = surface_with(&engine, plain);
    engine.render(FrameTime::now()).expect("plain render");
    let plain_pixels = pixels(&surface);

    value.set(transformed.clone());
    engine.render(FrameTime::now()).expect("rotated render");
    let rotated_pixels = pixels(&surface);
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
    engine.render(FrameTime::now()).expect("restored render");
    assert_eq!(plain_pixels, pixels(&surface));
}

#[test]
fn invalid_transform_is_an_error() {
    let (engine, font) = engine_and_font();
    for transform in [
        Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]),
        Affine::scale(0.0),
    ] {
        let run = run(
            font.id(),
            vec![glyph(12.25, 52.5, Some(transform))],
            GlyphStyle::Fill,
        );
        let (surface, _) = surface_with(&engine, run);
        match engine.render(FrameTime::now()) {
            Err(RenderError::Render(_)) => {}
            other => panic!("expected Render error, got {other:?}"),
        }
        drop(surface);
    }
}

#[test]
fn color_font_registers() {
    let Ok(bytes) = std::fs::read(COLR_FONT_PATH) else {
        eprintln!("text-colr font not checked out; skipping");
        return;
    };
    let engine = Engine::<Gpu>::new(GpuConfig::default()).expect("CPU engine");
    engine
        .font(FontSource::bytes(bytes))
        .expect("a COLR font registers");
}
