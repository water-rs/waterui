//! Native bitmap colour-font rendering and cache behavior.

use cherenkov::kurbo::{Affine, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Draw, Engine, EngineError, Extend, FontId, FontSource, FrameTime, Glyph, GlyphRun, GlyphStyle,
    ImageColorSpace, ImageData, ImageId, ImagePattern, Offscreen, OffscreenFormat, Paint, Rgba8,
    Sampling, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig};
use skrifa::MetadataProvider;
use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapStrikes, Origin};
use skrifa::raw::TableProvider;

const CBDT_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scenes/fonts/NotoColorEmojiSubset.ttf"
);
const SBIX_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../scenes/fonts/CherenkovSbixTest.ttf"
);
const OUTLINE_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../scenes/fonts/NotoSans.ttf");

split_fn! {
fn engine() -> Option<Engine<Gpu>> {
    match wait!(Engine::<Gpu>::new(GpuConfig::default())) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(error) => panic!("GPU initialization failed: {error}"),
    }
}
}

fn glyph_id(bytes: &[u8], character: char) -> u32 {
    skrifa::FontRef::from_index(bytes, 0)
        .expect("font")
        .charmap()
        .map(character)
        .expect("fixture glyph")
        .to_u32()
}

fn glyph_run(font: FontId, id: u32, size: f32) -> GlyphRun {
    GlyphRun {
        font,
        size,
        coords: Vec::new().into(),
        glyphs: vec![Glyph {
            id,
            x: 24.0,
            y: 112.0,
            transform: None,
        }]
        .into(),
        style: GlyphStyle::Fill,
    }
}

/// The single glyph of a [`GlyphRun`], mutable: `Arc::make_mut` clones
/// the shared storage first when the run was already cloned into a
/// binding.
fn glyph_mut(run: &mut GlyphRun) -> &mut Glyph {
    std::sync::Arc::make_mut(&mut run.glyphs)
        .first_mut()
        .expect("one-glyph run")
}

split_fn! {
fn unregistered_image_error(bytes: &[u8], character: char) -> Option<String> {
    let engine = wait!(engine())?;
    let font = engine
        .font(FontSource::bytes(bytes.to_vec()))
        .expect("register font");
    let run = glyph_run(font.id(), glyph_id(bytes, character), 48.0);
    let surface = wait!(engine
        .surface(Offscreen::new((160, 120), OffscreenFormat::LinearF16)))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.glyphs(
                run,
                Paint::Image(ImagePattern {
                    image: ImageId::new(u64::MAX),
                    transform: Affine::IDENTITY,
                    extend_x: Extend::Pad,
                    extend_y: Extend::Pad,
                    sampling: Sampling::Linear,
                }),
            );
        }));
    });
    match wait!(engine.render(FrameTime::now())) {
        Err(cherenkov::RenderError::Image(message)) => Some(message),
        Err(error) => panic!("expected unregistered-image error, got {error}"),
        Ok(_) => panic!("unregistered-image paint unexpectedly rendered"),
    }
}
}

fn image_source(
    bytes: &[u8],
    format: BitmapFormat,
    glyph_id: u32,
    ppem: f32,
    size: f32,
) -> (ImageData<Rgba8>, Rect) {
    let font = skrifa::FontRef::from_index(bytes, 0).expect("font");
    let strikes = BitmapStrikes::with_format(&font, format).expect("bitmap strikes");
    let strike = strikes
        .iter()
        .find(|strike| strike.ppem().to_bits() == ppem.to_bits())
        .expect("selected strike");
    let glyph = strike
        .get(skrifa::GlyphId::new(glyph_id))
        .expect("bitmap glyph");
    let BitmapData::Png(png_bytes) = &glyph.data else {
        panic!("fixture uses PNG payloads");
    };
    let mut png_decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    png_decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut png_reader = png_decoder.read_info().expect("PNG");
    let mut rgba_bytes = vec![0; png_reader.output_buffer_size().expect("PNG output size")];
    let info = png_reader.next_frame(&mut rgba_bytes).expect("PNG frame");
    rgba_bytes.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => rgba_bytes,
        png::ColorType::Rgb => rgba_bytes
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[1], pixel[2], 255])
            .collect(),
        png::ColorType::Grayscale => rgba_bytes
            .iter()
            .flat_map(|&gray| [gray, gray, gray, 255])
            .collect(),
        png::ColorType::GrayscaleAlpha => rgba_bytes
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|pixel| [pixel[0], pixel[0], pixel[0], pixel[1]])
            .collect(),
        other @ png::ColorType::Indexed => panic!("unexpected fixture PNG format {other:?}"),
    };
    let upem = f64::from(font.head().expect("head").units_per_em());
    let x0 = f64::from(glyph.bearing_x) / upem
        + f64::from(glyph.inner_bearing_x) / f64::from(glyph.ppem_x);
    let y = f64::from(glyph.bearing_y) / upem
        + f64::from(glyph.inner_bearing_y) / f64::from(glyph.ppem_y);
    let width = f64::from(glyph.width) / f64::from(glyph.ppem_x);
    let height = f64::from(glyph.height) / f64::from(glyph.ppem_y);
    let (y0, y1) = match glyph.placement_origin {
        Origin::TopLeft => (-y, -y + height),
        Origin::BottomLeft => (-y - height, -y),
    };
    let rect = Rect::new(
        f64::from(size) * x0,
        f64::from(size) * y0,
        f64::from(size) * (x0 + width),
        f64::from(size) * y1,
    );
    (
        ImageData::<Rgba8>::new(glyph.width, glyph.height, rgba)
            .expect("image data")
            .color_space(ImageColorSpace::Srgb),
        rect,
    )
}

split_fn! {
#[expect(
    clippy::too_many_arguments,
    reason = "one helper checks equivalence across font, strike, size, and transforms"
)]
fn equivalent(
    engine: &Engine<Gpu>,
    font: &cherenkov::Font,
    bytes: &[u8],
    format: BitmapFormat,
    gid: u32,
    ppem: f32,
    size: f32,
    ambient: Affine,
    glyph_transform: Option<Affine>,
) {
    let (image_data, rect) = image_source(bytes, format, gid, ppem, size);
    let image = engine.image(image_data).expect("reference image");
    let mut run = glyph_run(font.id(), gid, size);
    glyph_mut(&mut run).transform = glyph_transform;
    let placement = Affine::translate((24.0, 112.0)) * glyph_transform.unwrap_or(Affine::IDENTITY);
    let actual = wait!(engine
        .surface(Offscreen::new((320, 220), OffscreenFormat::LinearF16)))
        .expect("actual surface");
    let reference = wait!(engine
        .surface(Offscreen::new((320, 220), OffscreenFormat::LinearF16)))
        .expect("reference surface");
    actual.update(|tx| {
        tx[actual.root()].content(actual.record(|c| {
            c.transform(ambient, |c| {
                c.glyphs(run.clone(), WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
            });
        }));
    });
    reference.update(|tx| {
        tx[reference.root()].content(reference.record(|c| {
            c.transform(ambient, |c| {
                c.transform(placement, |c| {
                    c.image(image.id(), rect, Sampling::Linear);
                });
            });
        }));
    });
    wait!(engine.render(FrameTime::now())).expect("render");
    let actual_pixels = wait!(actual.readback()).expect("actual readback").pixels;
    let reference_pixels = wait!(reference.readback()).expect("reference readback").pixels;
    assert!(actual_pixels.iter().any(|pixel| pixel[3] > 0.0));
    assert_eq!(
        actual_pixels
            .iter()
            .map(|pixel| pixel.map(f32::to_bits))
            .collect::<Vec<_>>(),
        reference_pixels
            .iter()
            .map(|pixel| pixel.map(f32::to_bits))
            .collect::<Vec<_>>()
    );
}
}

split_test! {
fn cbdt_and_sbix_glyphs_match_image_draws() {
    let Some(engine) = wait!(engine()) else {
        return;
    };
    let cbdt = std::fs::read(CBDT_PATH).expect("CBDT fixture");
    let cbdt_id = glyph_id(&cbdt, '😀');
    let cbdt_font = engine
        .font(FontSource::bytes(cbdt.clone()))
        .expect("register CBDT font");
    let ambient_cbdt = Affine::translate((24.0, 12.0))
        * Affine::rotate(0.22)
        * Affine::scale_non_uniform(1.2, 0.9);
    wait!(equivalent(
        &engine,
        &cbdt_font,
        &cbdt,
        BitmapFormat::Cbdt,
        cbdt_id,
        109.0,
        48.0,
        Affine::IDENTITY,
        None,
    ));
    wait!(equivalent(
        &engine,
        &cbdt_font,
        &cbdt,
        BitmapFormat::Cbdt,
        cbdt_id,
        109.0,
        48.0,
        ambient_cbdt,
        None,
    ));
    let transformed =
        Affine::rotate(0.22) * Affine::skew(0.18, -0.12) * Affine::scale_non_uniform(1.2, 0.9);
    wait!(equivalent(
        &engine,
        &cbdt_font,
        &cbdt,
        BitmapFormat::Cbdt,
        cbdt_id,
        109.0,
        48.0,
        Affine::IDENTITY,
        Some(transformed),
    ));

    let sbix = std::fs::read(SBIX_PATH).expect("sbix fixture");
    let sbix_id = glyph_id(&sbix, '😀');
    let sbix_font = engine
        .font(FontSource::bytes(sbix.clone()))
        .expect("register sbix font");
    wait!(equivalent(
        &engine,
        &sbix_font,
        &sbix,
        BitmapFormat::Sbix,
        sbix_id,
        32.0,
        20.0,
        Affine::IDENTITY,
        None,
    ));
    wait!(equivalent(
        &engine,
        &sbix_font,
        &sbix,
        BitmapFormat::Sbix,
        sbix_id,
        96.0,
        48.0,
        Affine::IDENTITY,
        None,
    ));
    wait!(equivalent(
        &engine,
        &sbix_font,
        &sbix,
        BitmapFormat::Sbix,
        sbix_id,
        96.0,
        20.0,
        Affine::scale(2.0),
        None,
    ));
    wait!(equivalent(
        &engine,
        &sbix_font,
        &sbix,
        BitmapFormat::Sbix,
        sbix_id,
        96.0,
        20.0,
        Affine::IDENTITY,
        Some(Affine::scale(2.0)),
    ));
    wait!(equivalent(
        &engine,
        &sbix_font,
        &sbix,
        BitmapFormat::Sbix,
        sbix_id,
        96.0,
        48.0,
        Affine::IDENTITY,
        Some(transformed),
    ));
    wait!(equivalent(
        &engine,
        &sbix_font,
        &sbix,
        BitmapFormat::Sbix,
        sbix_id,
        96.0,
        20.0,
        Affine::scale(1.5),
        Some(Affine::scale_non_uniform(1.2, 0.9)),
    ));
}
}

split_test! {
/// The generated sbix fixture carries a strike PNG in every colour type
/// and bit depth the decoder accepts; each strike draws identically to
/// its decoded image.
fn sbix_strikes_cover_every_png_colour_type() {
    let Some(engine) = wait!(engine()) else {
        return;
    };
    let sbix = std::fs::read(SBIX_PATH).expect("sbix fixture");
    let sbix_font = engine
        .font(FontSource::bytes(sbix.clone()))
        .expect("register sbix font");
    let font = skrifa::FontRef::from_index(&sbix, 0).expect("font");
    let strikes = BitmapStrikes::with_format(&font, BitmapFormat::Sbix)
        .expect("bitmap strikes");
    let mut covered = std::collections::BTreeSet::new();
    for (ppem, size) in [(32.0_f32, 20.0_f32), (96.0_f32, 48.0_f32)] {
        let strike = strikes
            .iter()
            .find(|strike| strike.ppem().to_bits() == ppem.to_bits())
            .expect("selected strike");
        for character in ['☕', '⚠', '⚡', '❤', '😀'] {
            let gid = glyph_id(&sbix, character);
            let glyph = strike.get(skrifa::GlyphId::new(gid)).expect("bitmap glyph");
            let BitmapData::Png(png_bytes) = &glyph.data else {
                panic!("fixture uses PNG payloads");
            };
            let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
            let reader = decoder.read_info().expect("PNG");
            let info = reader.info();
            covered.insert((info.color_type as u8, info.bit_depth as u8));
            wait!(equivalent(
                &engine,
                &sbix_font,
                &sbix,
                BitmapFormat::Sbix,
                gid,
                ppem,
                size,
                Affine::IDENTITY,
                None,
            ));
        }
    }
    assert_eq!(
        covered,
        [
            (6, 8),
            (6, 16),
            (2, 8),
            (2, 16),
            (0, 8),
            (0, 16),
            (4, 8),
            (4, 16),
        ]
        .into_iter()
        .collect()
    );
}
}

split_test! {
fn bitmap_cache_reuses_transformed_glyphs_and_uses_font_identity() {
    let Some(engine) = wait!(engine()) else {
        return;
    };
    let bytes = std::fs::read(CBDT_PATH).expect("CBDT fixture");
    let small = glyph_id(&bytes, '☕');
    let large = glyph_id(&bytes, '😀');
    let first_font = engine
        .font(FontSource::bytes(bytes.clone()))
        .expect("first font");
    let second_font = engine.font(FontSource::bytes(bytes)).expect("second font");
    let transform = Affine::rotate(0.2) * Affine::scale_non_uniform(1.2, 0.8);
    let mut first_run = glyph_run(first_font.id(), small, 48.0);
    glyph_mut(&mut first_run).transform = Some(transform);
    let mut second_run = glyph_run(second_font.id(), small, 48.0);
    glyph_mut(&mut second_run).transform = Some(transform);
    let first = nami::Binding::container(first_run);
    let second = nami::Binding::container(second_run);
    let surface = wait!(engine
        .surface(Offscreen::new((240, 160), OffscreenFormat::LinearF16)))
        .expect("surface");
    let content = surface.record(|c| {
        c.glyphs(first.clone(), WorkingColor::WHITE);
        c.glyphs(second.clone(), WorkingColor::WHITE);
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    wait!(engine.render(FrameTime::now())).expect("initial frame");
    assert_eq!(engine.stats().glyphs_rasterized, 2);
    wait!(engine.render(FrameTime::now())).expect("identical frame");
    assert_eq!(engine.stats().glyphs_rasterized, 0);

    let mut changed = glyph_run(first_font.id(), large, 48.0);
    glyph_mut(&mut changed).x = 100.0;
    glyph_mut(&mut changed).transform = Some(transform);
    first.set(changed);
    wait!(engine.render(FrameTime::now())).expect("dirty glyph");
    assert_eq!(engine.stats().glyphs_rasterized, 1);

    let only_second = surface.record(|c| {
        c.glyphs(second.clone(), WorkingColor::WHITE);
    });
    surface.update(|tx| {
        tx[surface.root()].content(only_second);
    });
    drop(first_font);
    wait!(engine.render(FrameTime::now())).expect("font removal");
    assert_eq!(engine.stats().glyphs_rasterized, 0);
    assert!(
        wait!(surface
            .readback())
            .expect("readback")
            .pixels
            .iter()
            .any(|pixel| pixel[3] > 0.0)
    );
}
}

split_test! {
fn bitmap_cache_reuses_unchanged_glyphs_and_uses_font_identity() {
    let Some(engine) = wait!(engine()) else {
        return;
    };
    let bytes = std::fs::read(CBDT_PATH).expect("CBDT fixture");
    let small = glyph_id(&bytes, '☕');
    let large = glyph_id(&bytes, '😀');
    let first_font = engine
        .font(FontSource::bytes(bytes.clone()))
        .expect("first font");
    let second_font = engine.font(FontSource::bytes(bytes)).expect("second font");
    let first = nami::Binding::container(glyph_run(first_font.id(), small, 48.0));
    let second = nami::Binding::container(glyph_run(second_font.id(), small, 48.0));
    let surface = wait!(engine
        .surface(Offscreen::new((240, 160), OffscreenFormat::LinearF16)))
        .expect("surface");
    let content = surface.record(|c| {
        c.glyphs(first.clone(), WorkingColor::WHITE);
        c.glyphs(second.clone(), WorkingColor::WHITE);
    });
    surface.update(|tx| {
        tx[surface.root()].content(content);
    });
    wait!(engine.render(FrameTime::now())).expect("initial frame");
    assert_eq!(engine.stats().glyphs_rasterized, 2);
    wait!(engine.render(FrameTime::now())).expect("identical frame");
    assert_eq!(engine.stats().glyphs_rasterized, 0);

    let mut changed = glyph_run(first_font.id(), large, 48.0);
    glyph_mut(&mut changed).x = 100.0;
    first.set(changed);
    wait!(engine.render(FrameTime::now())).expect("dirty glyph");
    assert_eq!(engine.stats().glyphs_rasterized, 1);

    let only_second = surface.record(|c| {
        c.glyphs(second.clone(), WorkingColor::WHITE);
    });
    surface.update(|tx| {
        tx[surface.root()].content(only_second);
    });
    drop(first_font);
    wait!(engine.render(FrameTime::now())).expect("font removal");
    assert_eq!(engine.stats().glyphs_rasterized, 0);
    assert!(
        wait!(surface
            .readback())
            .expect("readback")
            .pixels
            .iter()
            .any(|pixel| pixel[3] > 0.0)
    );
}
}

split_test! {
fn missing_notdef_is_empty_and_bitmap_transforms_validate_and_strokes_are_unsupported() {
    let Some(engine) = wait!(engine()) else {
        return;
    };
    let bytes = std::fs::read(CBDT_PATH).expect("CBDT fixture");
    let font = engine
        .font(FontSource::bytes(bytes))
        .expect("register font");
    let surface = wait!(engine
        .surface(Offscreen::new((160, 120), OffscreenFormat::LinearF16)))
        .expect("surface");
    let mut missing = glyph_run(font.id(), 0, 48.0);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.glyphs(missing.clone(), WorkingColor::WHITE);
        }));
    });
    wait!(engine
        .render(FrameTime::now()))
        .expect("missing glyph frame");
    assert_eq!(engine.stats().glyphs_rasterized, 0);
    assert!(
        wait!(surface
            .readback())
            .expect("readback")
            .pixels
            .iter()
            .all(|pixel| pixel[3].to_bits() == 0)
    );

    glyph_mut(&mut missing).id = glyph_id(&std::fs::read(CBDT_PATH).expect("CBDT fixture"), '😀');
    glyph_mut(&mut missing).transform = Some(Affine::scale_non_uniform(0.0, 1.0));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.glyphs(missing.clone(), WorkingColor::WHITE);
        }));
    });
    assert!(matches!(
        wait!(engine.render(FrameTime::now())),
        Err(cherenkov::RenderError::Render(message))
            if message == "glyph transform must be finite and invertible"
    ));

    glyph_mut(&mut missing).transform = Some(Affine::rotate(0.2));
    missing.style = GlyphStyle::Stroke(cherenkov::kurbo::Stroke::new(1.0));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.glyphs(missing.clone(), WorkingColor::WHITE);
        }));
    });
    assert!(matches!(
        wait!(engine.render(FrameTime::now())),
        Err(cherenkov::RenderError::Unsupported("glyph-stroke"))
    ));
}
}

split_test! {
fn bitmap_runs_validate_image_paints_like_outline_runs() {
    let cbdt = std::fs::read(CBDT_PATH).expect("CBDT fixture");
    let outline = std::fs::read(OUTLINE_PATH).expect("outline fixture");
    let Some(bitmap_error) = wait!(unregistered_image_error(&cbdt, '😀')) else {
        return;
    };
    let Some(outline_error) = wait!(unregistered_image_error(&outline, 'A')) else {
        return;
    };
    assert_eq!(bitmap_error, outline_error);
}
}

split_fn! {
fn render_static_bitmap(
    engine: &Engine<Gpu>,
    font: FontId,
    glyph: u32,
    transform: Affine,
) -> Result<Vec<[f32; 4]>, Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.clear_color(WorkingColor::BLACK);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    let mut run = glyph_run(font, glyph, 48.0);
    glyph_mut(&mut run).x = 10.0;
    glyph_mut(&mut run).y = 56.0;
    surface.update(|tx| {
        tx[&layer]
            .transform(transform)
            .content(surface.record(|c| c.glyphs(run, WorkingColor::WHITE)));
    });
    wait!(engine.render(FrameTime::now()))?;
    Ok(wait!(surface.readback())?.pixels)
}
}

fn bitmap_pixel_bits(pixels: &[[f32; 4]]) -> Vec<[u32; 4]> {
    pixels.iter().map(|pixel| pixel.map(f32::to_bits)).collect()
}

split_test! {
/// Animated bitmap placement follows the layer's quarter-pixel snap and
/// returns to exact placement when the animation settles.
fn an_animating_layer_places_bitmap_glyphs_on_the_quarter_pixel_grid()
-> Result<(), Box<dyn std::error::Error>> {
    use nami::SignalExt as _;
    use std::time::Duration;
use cherenkov::Instant;

    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let bytes = std::fs::read(SBIX_PATH)?;
    let glyph = glyph_id(&bytes, '😀');
    let font = engine.font(FontSource::bytes(bytes))?;
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.clear_color(WorkingColor::BLACK);
    let layer = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&layer);
    });
    let mut run = glyph_run(font.id(), glyph, 48.0);
    glyph_mut(&mut run).x = 10.0;
    glyph_mut(&mut run).y = 56.0;
    surface.update(|tx| {
        tx[&layer].content(surface.record(|c| c.glyphs(run, WorkingColor::WHITE)));
    });
    let translate = nami::binding(Affine::IDENTITY);
    surface.update(|tx| {
        tx[&layer].transform(translate.clone().with(cherenkov::Animation::from(
            cherenkov::Curve::linear(Duration::from_secs(1)),
        )));
    });

    let start = Instant::now();
    translate.set(Affine::translate((1.2, 0.0)));
    wait!(engine.render(FrameTime::at(start)))?;
    let at_start = wait!(surface.readback())?.pixels;
    assert!(matches!(
        wait!(engine.render(FrameTime::at(start + Duration::from_millis(250))))?,
        cherenkov::Next::At { .. }
    ));
    let quarter = wait!(surface.readback())?.pixels;
    assert_eq!(
        wait!(engine.render(FrameTime::at(start + Duration::from_secs(2))))?,
        cherenkov::Next::Idle
    );
    let settled = wait!(surface.readback())?.pixels;

    let identity = wait!(render_static_bitmap(&engine, font.id(), glyph, Affine::IDENTITY))?;
    let quarter_static =
        wait!(render_static_bitmap(&engine, font.id(), glyph, Affine::translate((0.25, 0.0))))?;
    let off_grid = wait!(render_static_bitmap(&engine, font.id(), glyph, Affine::translate((0.3, 0.0))))?;
    let settled_static =
        wait!(render_static_bitmap(&engine, font.id(), glyph, Affine::translate((1.2, 0.0))))?;
    let off_settled =
        wait!(render_static_bitmap(&engine, font.id(), glyph, Affine::translate((1.25, 0.0))))?;
    assert_eq!(bitmap_pixel_bits(&at_start), bitmap_pixel_bits(&identity));
    assert_eq!(
        bitmap_pixel_bits(&quarter),
        bitmap_pixel_bits(&quarter_static)
    );
    assert_ne!(bitmap_pixel_bits(&quarter), bitmap_pixel_bits(&off_grid));
    assert_eq!(
        bitmap_pixel_bits(&settled),
        bitmap_pixel_bits(&settled_static)
    );
    assert_ne!(bitmap_pixel_bits(&settled), bitmap_pixel_bits(&off_settled));
    Ok(())
}
}
