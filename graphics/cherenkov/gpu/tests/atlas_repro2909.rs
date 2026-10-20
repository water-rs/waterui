//! water-rs/waterui#2909: a text-heavy 1400×3200 mount must grow,
//! shrink and evict the glyph atlas — never `AtlasExhausted`. The
//! commit honors every verified `Resize` verdict and the replay pins
//! cover every band an emission samples, so an ordinary batch always
//! finds room.

use cherenkov::kurbo::{BezPath, Circle, RoundedRect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Budget, Bytes, Draw, Engine, EngineError, GlyphRun, Offscreen, OffscreenFormat, WorkingColor,
};
use cherenkov_gpu::{Gpu, GpuConfig};

const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../scenes/fonts/NotoSans.ttf");

split_fn! {
fn engine(config: GpuConfig) -> Option<Engine<Gpu>> {
    match wait!(Engine::<Gpu>::new(config)) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(e) => panic!("engine init failed: {e}"),
    }
}
}

fn font() -> cherenkov::FontSource {
    cherenkov::FontSource::bytes(std::fs::read(FONT_PATH).expect("scenes/fonts/NotoSans.ttf"))
}

/// `count` glyphs, each at a distinct fractional position → distinct cell keys.
/// Position math runs in `f64` (u32 keys convert losslessly); the `Glyph`
/// fields are `f32` API fields, the one narrowing this function makes.
#[allow(clippy::cast_possible_truncation)]
fn heavy_run(font: cherenkov::FontId, base_y: f64, count: u32, size: f32, salt: u32) -> GlyphRun {
    let size64 = f64::from(size);
    let glyphs: Vec<_> = (0..count)
        .map(|i| {
            let k = i + salt;
            let kf = f64::from(k);
            cherenkov::Glyph {
                id: 1 + k % 200,
                x: (size64 * 0.62).mul_add(f64::from(k % 60), kf * 0.37) as f32,
                y: (size64 * 1.35).mul_add(f64::from(k / 60), kf.mul_add(0.13, base_y)) as f32,
                transform: None,
            }
        })
        .collect();
    GlyphRun {
        font,
        size,
        coords: Vec::new().into(),
        glyphs: glyphs.into(),
        style: cherenkov::GlyphStyle::Fill,
    }
}

fn polygon(points: &[(f64, f64)]) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(points[0]);
    for &p in &points[1..] {
        path.line_to(p);
    }
    path.close_path();
    path
}

split_test! {
/// 1400×3200, ~3000 distinct-position glyphs + chrome paths (#2909).
fn huge_text_frame_grows_atlas() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let font = engine.font(font()).expect("font");
    let surface = wait!(engine.surface(
        Offscreen::new((1400, 3200), OffscreenFormat::LinearF16),
        || {}
    ))?;
    let ink = WorkingColor::new([0.4, 0.4, 0.5, 1.0]);
    let bubble = WorkingColor::new([0.9, 0.9, 0.95, 1.0]);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            for row in 0..25u32 {
                let y = f64::from(row).mul_add(120.0, 30.0);
                c.fill(Circle::new((40.0, y + 40.0), 24.0), ink);
                c.fill(RoundedRect::new(80.0, y, 800.0, y + 104.0, 18.0), bubble);
                c.glyphs(heavy_run(font.id(), y + 40.0, 40, 17.0, row * 40), ink);
                c.glyphs(heavy_run(font.id(), y + 74.0, 60, 14.0, row * 61), ink);
            }
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    for _ in 0..4 {
        wait!(engine.render(cherenkov::FrameTime::now()))?;
    }
    Ok(())
}
}

split_test! {
/// Big→small→big content churn at a tight atlas budget: each frame
/// re-lowers with a different live set, so the commit repeatedly grows,
/// shrinks and evicts. No frame may panic or report AtlasExhausted.
fn churning_batches_never_exhaust() -> Result<(), Box<dyn std::error::Error>> {
    let config = GpuConfig {
        budget: Budget {
            gpu: Bytes::mib(64),
            ..Budget::default()
        },
        ..GpuConfig::default()
    };
    let Some(engine) = wait!(engine(config)) else {
        return Ok(());
    };
    let font = engine.font(font()).expect("font");
    let surface = wait!(engine.surface(
        Offscreen::new((1400, 3200), OffscreenFormat::LinearF16),
        || {}
    ))?;
    let ink = WorkingColor::new([0.4, 0.4, 0.5, 1.0]);
    let bubble = WorkingColor::new([0.9, 0.9, 0.95, 1.0]);
    for frame in 0..8u32 {
        let rows = 4 + (frame % 3) * 10;
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                for row in 0..rows {
                    let y = f64::from(row).mul_add(120.0, 30.0);
                    let w = f64::from(row).mul_add(9.0, 700.0).min(1300.0);
                    c.fill(Circle::new((40.0, y + 40.0), 24.0), ink);
                    c.fill(
                        polygon(&[
                            (80.0, y),
                            (80.0 + w, y),
                            (80.0 + w, y + 104.0),
                            (80.0, y + 104.0),
                        ]),
                        bubble,
                    );
                    c.glyphs(
                        heavy_run(font.id(), y + 40.0, 40, 17.0, frame * 977 + row * 40),
                        ink,
                    );
                    c.glyphs(
                        heavy_run(font.id(), y + 74.0, 60, 14.0, frame * 613 + row * 61),
                        ink,
                    );
                }
            }));
        });
        wait!(engine.render(cherenkov::FrameTime::now()))?;
    }
    Ok(())
}
}
