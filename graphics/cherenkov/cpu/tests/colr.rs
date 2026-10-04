//! COLR colour glyph expansion, foreground resolution and unsupported
//! per-glyph transforms.
#![cfg(not(target_arch = "wasm32"))]
use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{
    Draw, Engine, Extend, FontSource, FrameTime, Glyph, GlyphRun, GlyphStyle, ImageData,
    ImagePattern, Offscreen, OffscreenFormat, Rgba8, Sampling, WorkingColor,
};
use cherenkov_cpu::{Raster, RasterConfig};
use nami::Binding;

/// Glyph ids in `scenes/fonts/CherenkovColrTest.ttf` (the glyph order is
/// fixed by `build_colr_test_font.py`): `.notdef box disc discL discR
/// cross` then the colour base glyphs in codepoint order.
const G_E300: u32 = 45; // foreground solid, alpha 1.0
const G_E301: u32 = 46; // foreground solid, alpha 0.4

fn colr_font(engine: &Engine<Raster>) -> cherenkov::Font {
    engine
        .font(FontSource::bytes(
            std::fs::read("../scenes/fonts/CherenkovColrTest.ttf").expect("test font"),
        ))
        .expect("registered font")
}

fn run(font: cherenkov::FontId, glyphs: Vec<Glyph>) -> GlyphRun {
    GlyphRun {
        font,
        size: 64.0,
        coords: Vec::new().into(),
        glyphs: glyphs.into(),
        style: GlyphStyle::Fill,
    }
}

const fn centre(pixels: &[[f32; 4]], width: usize, x: usize, y: usize) -> [f32; 4] {
    pixels[y * width + x]
}

#[test]
fn colr_glyph_renders_and_run_paint_recaches_pixels() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let font = colr_font(&engine);
    let surface = engine
        .surface(Offscreen::new((96, 72), OffscreenFormat::LinearF32))
        .expect("surface");
    // gE300's disc centre lands near (40, 35) at size 64, pad (8, 64).
    let glyph = Glyph {
        id: G_E300,
        x: 8.0,
        y: 64.0,
        transform: None,
    };
    let value = Binding::container(run(font.id(), vec![glyph]));
    let content = surface.record(|c| {
        c.fill(
            Rect::new(0.0, 0.0, 96.0, 72.0),
            WorkingColor::new([0.94, 0.94, 0.94, 1.0]),
        );
        c.glyphs(value.clone(), WorkingColor::new([0.04, 0.35, 0.78, 1.0]));
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    engine.render(FrameTime::now()).expect("first frame");
    let first = surface.readback().expect("first pixels").pixels;
    // The disc centre carries the run paint; a corner stays background.
    let corner = centre(&first, 96, 4, 4);
    let mid = centre(&first, 96, 40, 35);
    assert!(
        mid.map(f32::to_bits) != corner.map(f32::to_bits),
        "COLR glyph painted nothing at the disc centre"
    );
    // A different run paint must produce different pixels on the glyph
    // even though the cached node tree is run-independent.
    value.set(run(font.id(), vec![glyph]));
    let second_paint = surface.record(|c| {
        c.fill(
            Rect::new(0.0, 0.0, 96.0, 72.0),
            WorkingColor::new([0.94, 0.94, 0.94, 1.0]),
        );
        c.glyphs(value.clone(), WorkingColor::new([0.78, 0.04, 0.04, 1.0]));
    });
    surface.update(|tx| {
        tx[surface.root()].content(second_paint);
    });
    engine.render(FrameTime::now()).expect("second frame");
    let second = surface.readback().expect("second pixels").pixels;
    assert!(
        centre(&second, 96, 40, 35).map(f32::to_bits) != mid.map(f32::to_bits),
        "run paint change did not reach the COLR glyph"
    );
}

#[test]
fn image_foreground_alpha_scales_group_opacity() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let font = colr_font(&engine);
    // A 1x1 solid image: the foreground paint is an image without an
    // alpha channel, so the COLR alpha 0.4 must land as group opacity.
    let image = engine
        .image(ImageData::<Rgba8>::new(1, 1, vec![40, 80, 200, 255]).expect("data"))
        .expect("image");
    let surface = engine
        .surface(Offscreen::new((96, 72), OffscreenFormat::LinearF32))
        .expect("surface");
    let bg = WorkingColor::WHITE;
    // Render each glyph (alpha 1.0 and 0.4) with the image foreground.
    let paint = cherenkov::Paint::Image(ImagePattern {
        image: image.id(),
        transform: Affine::IDENTITY,
        extend_x: Extend::Pad,
        extend_y: Extend::Pad,
        sampling: Sampling::Nearest,
    });
    let binding = Binding::container(run(
        font.id(),
        vec![Glyph {
            id: G_E300,
            x: 8.0,
            y: 64.0,
            transform: None,
        }],
    ));
    let content = surface.record(|c| {
        c.fill(Rect::new(0.0, 0.0, 96.0, 72.0), bg);
        c.glyphs(binding.clone(), paint.clone());
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    engine.render(FrameTime::now()).expect("alpha 1 frame");
    let full = centre(&surface.readback().expect("full pixels").pixels, 96, 40, 35);
    binding.set(run(
        font.id(),
        vec![Glyph {
            id: G_E301,
            x: 8.0,
            y: 64.0,
            transform: None,
        }],
    ));
    // The binding only swaps the run; re-record so the picture carries
    // the same image paint over the same background.
    let faded = surface.record(|c| {
        c.fill(Rect::new(0.0, 0.0, 96.0, 72.0), bg);
        c.glyphs(binding.clone(), paint);
    });
    surface.update(|tx| {
        tx[surface.root()].content(faded);
    });
    engine.render(FrameTime::now()).expect("alpha 0.4 frame");
    let soft = centre(&surface.readback().expect("soft pixels").pixels, 96, 40, 35);
    for (c, (soft, full)) in soft.iter().zip(full.iter()).enumerate() {
        let expected = 1.0 - 0.4 + 0.4 * full;
        assert!(
            (soft - expected).abs() < 1e-3,
            "channel {c}: expected {expected}, got {soft} (full {full})"
        );
    }
}

#[test]
fn per_glyph_transform_rotates_colr_glyph() {
    let engine = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let font = colr_font(&engine);
    let surface = engine
        .surface(Offscreen::new((96, 72), OffscreenFormat::LinearF32))
        .expect("surface");
    let render = |transform| {
        let content = surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 96.0, 72.0), WorkingColor::WHITE);
            c.glyphs(
                run(
                    font.id(),
                    vec![Glyph {
                        id: G_E300,
                        x: 32.0,
                        y: 64.0,
                        transform,
                    }],
                ),
                WorkingColor::BLACK,
            );
        });
        surface.update(|tx| {
            tx[surface.root()].content(content);
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("pixels").pixels
    };
    let plain = render(None);
    let rotated = render(Some(Affine::rotate(0.2)));
    assert!(
        rotated.iter().any(|pixel| pixel[0] < 1.0),
        "rotated COLR glyph should render non-empty pixels"
    );
    assert_ne!(
        plain, rotated,
        "the per-glyph transform should change the rendered COLR glyph"
    );
}
