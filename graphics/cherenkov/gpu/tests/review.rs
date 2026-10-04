//! Tests for the slice-1 review fixes: atlas growth as a result, the
//! clear-only dirty flag, dropped-surface cleanup, the oracle-exact radial
//! gradient parameter and the zero-size surface error.

use cherenkov::kurbo::{Point, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{
    Budget, Bytes, Engine, EngineError, Offscreen, OffscreenFormat, RenderError, SurfaceError,
};
use cherenkov::{Draw, GlyphRun, WorkingColor};
use cherenkov_gpu::{Gpu, GpuConfig, TimestampSupport};

split_fn! {
/// An engine under `config`, or `None` when no adapter exists.
fn engine(config: GpuConfig) -> Option<Engine<Gpu>> {
    match wait!(Engine::<Gpu>::new(config)) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(e) => panic!("engine init failed: {e}"),
    }
}
}

/// The committed corpus subset of Noto Sans (never a host system font).
const FONT_PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../scenes/fonts/NotoSans.ttf");

/// Glyph ids the subset has outlines for.
const FONT_GLYPHS: u32 = 200;

split_fn! {
fn make(
    engine: &Engine<Gpu>,
    font: cherenkov::FontId,
    color: WorkingColor,
) -> Result<cherenkov::Surface<Gpu>, Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(8.0, 8.0, 56.0, 56.0), color);
            // Shared atlas contention: both surfaces raster the same
            // glyphs on their own thread.
            for run in text_runs(font, 96, 10.0) {
                c.glyphs(run, WorkingColor::WHITE);
            }
        }));
    });
    Ok(surface)
}
}

fn font() -> cherenkov::FontSource {
    cherenkov::FontSource::bytes(std::fs::read(FONT_PATH).expect("scenes/fonts/NotoSans.ttf"))
}

/// `count` distinct glyph-cache entries at `size` px, tiled on a grid: the
/// glyph ids cycle through the subset, and each cycle steps the size so no
/// two entries share an atlas cell.
#[expect(clippy::cast_precision_loss)]
fn text_runs(font: cherenkov::FontId, count: u32, size: f32) -> Vec<GlyphRun> {
    (0..count.div_ceil(FONT_GLYPHS))
        .map(|cycle| {
            let size = (cycle as f32).mul_add(2.0, size);
            let glyphs: Vec<_> = (cycle * FONT_GLYPHS
                ..(cycle * FONT_GLYPHS + FONT_GLYPHS).min(count))
                .map(|i| cherenkov::Glyph {
                    id: 1 + i % FONT_GLYPHS,
                    x: (i % 32) as f32 * (size * 0.8),
                    y: (1 + i / 32) as f32 * size,
                    transform: None,
                })
                .collect();
            GlyphRun {
                font,
                size,
                coords: Vec::new().into(),
                glyphs: glyphs.into(),
                style: cherenkov::GlyphStyle::Fill,
            }
        })
        .collect()
}

split_fn! {
fn render_text(
    config: GpuConfig,
    count: u32,
    size: f32,
) -> Option<Result<Vec<[f32; 4]>, RenderError>> {
    let engine = wait!(engine(config))?;
    let font = engine.font(font()).expect("font");
    let surface = wait!(engine
        .surface(Offscreen::new((512, 512), OffscreenFormat::LinearF16)))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            for run in text_runs(font.id(), count, size) {
                c.glyphs(run, WorkingColor::WHITE);
            }
        }));
    });
    Some(match wait!(engine.render(cherenkov::FrameTime::now())) {
        Ok(_) => wait!(surface.readback()).map(|r| r.pixels),
        Err(e) => Err(e),
    })
}
}

split_test! {
/// A 128-glyph run renders identically whether the atlas can grow or is
/// capped at its start size; a 2000-glyph live set overflows the cap.
fn a_full_atlas_grows_then_reports_exhaustion() -> Result<(), Box<dyn std::error::Error>> {
    let tiny = GpuConfig {
        budget: Budget {
            gpu: Bytes::mib(1),
            ..Budget::default()
        },
        ..GpuConfig::default()
    };
    let (Some(big), Some(small)) = (
        wait!(render_text(GpuConfig::default(), 128, 48.0)),
        wait!(render_text(tiny.clone(), 128, 48.0)),
    ) else {
        return Ok(());
    };
    let big = big?;
    let small = small?;
    assert!(big.iter().any(|p| p[3] > 0.0), "default render is empty");
    assert!(small.iter().any(|p| p[3] > 0.0), "capped render is empty");
    let eps = 1.0 / 255.0 + 1e-4;
    for (i, (a, b)) in big.iter().zip(&small).enumerate() {
        for c in 0..4 {
            assert!(
                (a[c] - b[c]).abs() <= eps,
                "pixel {i} channel {c}: {} vs {}",
                a[c],
                b[c]
            );
        }
    }
    // A live set that does not fit the capped atlas exhausts it.
    let Some(exhausted) = wait!(render_text(tiny, 2000, 48.0)) else {
        return Ok(());
    };
    assert!(
        matches!(&exhausted, Err(RenderError::AtlasExhausted)),
        "expected AtlasExhausted, got {exhausted:?}"
    );
    Ok(())
}
}

split_test! {
/// A clear-colour-only commit marks the surface dirty and re-renders.
fn a_clear_only_commit_renders() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((16, 16), OffscreenFormat::LinearF16)))?;
    surface.clear_color(WorkingColor::new([1.0, 0.0, 0.0, 1.0]));
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert!(wait!(surface.readback())?.pixels[0][0] > 0.9, "red clear");
    surface.clear_color(WorkingColor::new([0.0, 0.0, 1.0, 1.0]));
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let [r, g, b, a] = wait!(surface.readback())?.pixels[0];
    assert!(
        b > 0.9 && r < 0.1 && g < 0.1 && a > 0.9,
        "blue clear: {r} {g} {b} {a}"
    );
    Ok(())
}
}

split_test! {
/// Dropping a surface releases its engine entry and GPU textures.
fn dropped_surfaces_do_not_leak() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let before = wait!(engine.memory()).gpu;
    for _ in 0..200 {
        drop(wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?);
    }
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(engine.live_surfaces(), 0, "surfaces still live");
    assert_eq!(wait!(engine.memory()).gpu, before, "gpu memory grew");
    Ok(())
}
}

/// The oracle's `radial_t`, ported for the cone test.
fn oracle_t(p: (f64, f64), c0: (f64, f64), r0: f64, c1: (f64, f64), r1: f64) -> f64 {
    let (px, py) = (p.0 - c0.0, p.1 - c0.1);
    let (dcx, dcy) = (c1.0 - c0.0, c1.1 - c0.1);
    let dr = r1 - r0;
    let a = dr.mul_add(-dr, dcy.mul_add(dcy, dcx * dcx));
    let b = -2.0 * r0.mul_add(dr, dcy.mul_add(py, dcx * px));
    let c = r0.mul_add(-r0, py.mul_add(py, px * px));
    if a.abs() < 1e-12 {
        if b.abs() < 1e-12 {
            return if r0.abs() < 1e-12 {
                0.0
            } else {
                (px.hypot(py) - r0) / r0.abs()
            };
        }
        return -c / b;
    }
    let disc = (4.0 * a).mul_add(-c, b * b);
    if disc < 0.0 {
        return f64::NAN;
    }
    let sq = disc.sqrt();
    ((-b + sq) / (2.0 * a)).max((-b - sq) / (2.0 * a))
}

fn radial_fill(
    center0: (f64, f64),
    r0: f64,
    center1: (f64, f64),
    r1: f64,
) -> cherenkov::RadialGradient {
    cherenkov::RadialGradient {
        start_center: Point::new(center0.0, center0.1),
        start_radius: r0,
        end_center: Point::new(center1.0, center1.1),
        end_radius: r1,
        stops: vec![
            cherenkov::ColorStop {
                offset: 0.0,
                color: WorkingColor::new([0.0, 0.0, 0.0, 1.0]),
            },
            cherenkov::ColorStop {
                offset: 1.0,
                color: WorkingColor::new([1.0, 1.0, 1.0, 1.0]),
            },
        ],
        extend: cherenkov::Extend::Pad,
        interpolation: cherenkov::Interpolation::Working,
    }
}

split_fn! {
fn render_radial(gradient: cherenkov::RadialGradient) -> Option<Vec<[f32; 4]>> {
    let engine = wait!(engine(GpuConfig::default()))?;
    let surface = wait!(engine
        .surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16)))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 128.0, 128.0),
                cherenkov::Paint::Radial(gradient),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now())).expect("render");
    Some(wait!(surface.readback()).expect("readback").pixels)
}
}

split_test! {
/// Coincident circles (c0 == c1, r0 == r1) interpolate on distance / r0.
fn coincident_circles_interpolate_by_distance() {
    let Some(pixels) = wait!(render_radial(radial_fill((64., 64.), 20., (64., 64.), 20.))) else {
        return;
    };
    let at = |x: usize, y: usize| pixels[y * 128 + x];
    // Pixel centres land at half-integer coordinates: distance 30.5 gives
    // t = (30.5 - 20) / 20 = 0.525.
    let px = at(94, 64);
    assert!((px[0] - 0.525).abs() < 5e-3, "t=0.525 pixel: {px:?}");
    // Distance 10 is inside r0: t < 0 pads to the first stop.
    let inside = at(74, 64);
    assert!(inside[0] < 1e-3, "inside pixel: {inside:?}");
}
}

split_test! {
/// A cone where the quadratic's negative-radius branch matters: the shader
/// follows the oracle, including NaN outside the cone.
#[expect(clippy::cast_precision_loss)]
#[expect(clippy::cast_possible_truncation)]
fn a_cone_gradient_matches_the_oracle() {
    let c0 = (32., 64.);
    let c1 = (72., 64.);
    let Some(pixels) = wait!(render_radial(radial_fill(c0, 0., c1, 30.))) else {
        return;
    };
    let at = |x: usize, y: usize| pixels[y * 128 + x];
    // Pixels where the oracle's discriminant is negative get NaN →
    // transparent.
    for (x, y) in [(32, 20), (10, 20), (32, 44)] {
        let t = oracle_t((x as f64 + 0.5, y as f64 + 0.5), c0, 0., c1, 30.);
        assert!(t.is_nan(), "expected NaN at ({x}, {y}), got {t}");
        assert!(
            at(x, y)[3] < 1e-6,
            "outside pixel ({x}, {y}): {:?}",
            at(x, y)
        );
    }
    // Inside the cone the shader's t equals the oracle's (clamped to the
    // stop range by the pad extension).
    for (x, y) in [(33, 64), (35, 64), (80, 44)] {
        let t = oracle_t((x as f64 + 0.5, y as f64 + 0.5), c0, 0., c1, 30.);
        let want = t.clamp(0.0, 1.0) as f32;
        assert!(
            (at(x, y)[0] - want).abs() < 5e-2,
            "cone pixel ({x}, {y}) t={t}: {:?}",
            at(x, y)
        );
    }
}
}

split_test! {
/// Dropping the last `Font` clone frees the renderer's font state once no
/// installed content draws the font (#199): until then frames that
/// consult it still draw; afterwards the atlas's glyph cells are purged and
/// content installed later that names the id fails fast instead of silently
/// keeping the font alive.
fn dropping_a_font_frees_its_renderer_state() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let font = engine.font(font())?;
    let font_id = font.id();
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let solid = |surface: &cherenkov::Surface<Gpu>| {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 64.0, 64.0), WorkingColor::WHITE);
            }));
        });
    };
    solid(&surface);
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let without_glyphs = wait!(engine.memory()).cpu;
    let runs = text_runs(font_id, 8, 24.0);
    let text = |surface: &cherenkov::Surface<Gpu>| {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.glyphs(runs[0].clone(), WorkingColor::WHITE);
            }));
        });
    };
    text(&surface);
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert!(wait!(engine.memory()).cpu > without_glyphs, "glyph cells cached");
    drop(font);
    // Dirty the surface so the next frame re-lowers and consults the font.
    surface.clear_color(WorkingColor::new([0.0, 0.0, 0.0, 1.0]));
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert!(
        wait!(engine.memory()).cpu > without_glyphs,
        "the installed content still draws the font"
    );
    solid(&surface);
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(wait!(engine.memory()).cpu, without_glyphs, "font cells released");
    text(&surface);
    assert!(
        matches!(
            wait!(engine.render(cherenkov::FrameTime::now())),
            Err(RenderError::Font(_))
        ),
        "content installed after the font was freed"
    );
    Ok(())
}
}

split_test! {
/// Timestamp queries accumulate on the renderer and are returned once,
/// oldest first, by `finish_timings`.
fn finish_timings_returns_every_drawn_frame_once() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig {
        timestamps: true,
        ..GpuConfig::default()
    })) else {
        return Ok(());
    };
    // An adapter without timestamp queries (the Apple Paravirtual device
    // on hosted macOS runners) never reports GPU timing, by contract.
    if engine.info().timestamps == TimestampSupport::Unsupported {
        return Ok(());
    }
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let mut submitted = Vec::new();
    for frame in 0..3u8 {
        let color = [f32::from(frame) / 3.0, 0.0, 0.0, 1.0];
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 64.0, 64.0), WorkingColor::new(color));
            }));
        });
        wait!(engine.render(cherenkov::FrameTime::now()))?;
        submitted.push(engine.stats().frame.expect("a drawing render submits"));
    }
    // An idle render has no frame id and cannot contribute another timing.
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let second = engine.stats();
    assert_eq!(second.frame, None, "nothing dirty, nothing submitted");
    let timings = wait!(engine.finish_timings())?;
    assert_eq!(
        timings.iter().map(|t| t.frame).collect::<Vec<_>>(),
        submitted,
        "every drawn frame is timed exactly once in increasing order"
    );
    assert!(timings[0].gpu_seconds.is_some());
    assert!(
        timings[0].passes.iter().any(|p| p.name == "surface"),
        "per-pass timing survives the deferred resolve"
    );
    assert!(
        wait!(engine.finish_timings())?.is_empty(),
        "nothing is outstanding once every timing is reported"
    );
    Ok(())
}
}

split_test! {
/// With timestamps disabled `finish_timings` reports no timing — and the
/// render never allocates per-pass metadata only the timestamp path consumes.
fn timestamps_off_reports_no_passes() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let stats = engine.stats();
    assert!(stats.frame.is_some());
    assert!(wait!(engine.finish_timings())?.is_empty());
    Ok(())
}
}

split_test! {
/// Re-rendering an unchanged scene creates no group-1 bind groups: the
/// second frame binds the views cached under (scratch generation, image
/// generation) rather than rebuilding them per encode.
fn bind_groups_are_reused_across_frames() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let record = |c: &mut cherenkov::Recorder| {
        c.fill(
            Rect::new(0.0, 0.0, 64.0, 64.0),
            WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
        );
        // An isolated group with overlapping contents forces a scratch
        // pass → a source-texture bind group.
        c.group(cherenkov::Group::new().opacity(0.5), |c| {
            c.fill(
                Rect::new(8.0, 8.0, 32.0, 32.0),
                WorkingColor::new([0.0, 0.0, 1.0, 1.0]),
            );
            c.fill(
                Rect::new(16.0, 16.0, 48.0, 48.0),
                WorkingColor::new([0.0, 1.0, 0.0, 1.0]),
            );
        });
    };
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| record(c)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert!(
        engine.stats().bind_groups_created > 0,
        "the first frame builds the bind groups"
    );
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| record(c)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(
        engine.stats().bind_groups_created,
        0,
        "an identical frame reuses the cached bind groups"
    );
    Ok(())
}
}

split_test! {
/// `Pressure::Moderate` evicts scratch/backdrop textures and cached plans; `Critical`
/// additionally returns the grow-only shared buffers to baseline —
/// `Engine::memory` shows the drop.
fn trim_releases_cached_memory() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    // Isolated group: needs a composition plan and grows the instance buffer.
    let scene = |c: &mut cherenkov::Recorder| {
        c.fill(
            Rect::new(0.0, 0.0, 64.0, 64.0),
            WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
        );
        c.group(cherenkov::Group::new().opacity(0.5), |c| {
            for i in 0..300 {
                c.fill(
                    Rect::new(f64::from(i % 20), f64::from(i % 20), 60.0, 60.0),
                    WorkingColor::new([0.0, 0.0, 1.0, 0.5]),
                );
            }
        });
    };
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| scene(c)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let before = wait!(engine.memory());
    engine.trim(cherenkov::Pressure::Moderate);
    let moderate = wait!(engine.memory());
    assert!(
        moderate.gpu.0 + moderate.cpu.0 < before.gpu.0 + before.cpu.0,
        "Moderate evicts temporary textures and plans: {} → {}",
        before.gpu.0 + before.cpu.0,
        moderate.gpu.0 + moderate.cpu.0
    );
    // Re-render so the instance buffer grows back, then trim hard.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| scene(c)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    engine.trim(cherenkov::Pressure::Critical);
    let critical = wait!(engine.memory());
    assert!(
        critical.gpu.0 <= moderate.gpu.0,
        "Critical also frees buffers: {} → {}",
        moderate.gpu.0,
        critical.gpu.0
    );
    // The engine still renders correctly afterwards — this encode runs
    // against the shrunk buffers and regrown scratch.
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| scene(c)));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let rb = wait!(surface.readback())?;
    assert!(
        rb.pixels[20 * 64 + 30][3] > 0.5,
        "post-trim frame still draws"
    );
    Ok(())
}
}

split_test! {
/// `render` returns `Next::Idle` until animation scheduling exists —
/// the contract the `Next::At` variant's docs state.
fn render_returns_idle() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(0.0, 0.0, 64.0, 64.0),
                WorkingColor::new([1.0, 0.0, 0.0, 1.0]),
            );
        }));
    });
    assert_eq!(
        wait!(engine.render(cherenkov::FrameTime::now()))?,
        cherenkov::Next::Idle
    );
    Ok(())
}
}

split_test! {
/// A zero-size surface is rejected synchronously.
fn a_zero_size_surface_is_an_error() {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return;
    };
    for size in [(0, 64), (64, 0), (0, 0)] {
        let result = wait!(engine.surface(Offscreen::new(size, OffscreenFormat::LinearF16)));
        assert!(
            matches!(result, Err(SurfaceError::ZeroSize)),
            "{size:?}: {result:?}"
        );
    }
}
}

split_test! {
/// Two surfaces dirtied in one frame lower on parallel workers — shared
/// glyph-atlas and font caches included — and each still renders its own
/// content correctly.
fn two_dirty_surfaces_lower_in_parallel() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine(GpuConfig::default())) else {
        return Ok(());
    };
    let font = engine.font(font())?;
    let red = wait!(make(&engine, font.id(), WorkingColor::new([1.0, 0.0, 0.0, 1.0])))?;
    let blue = wait!(make(&engine, font.id(), WorkingColor::new([0.0, 0.0, 1.0, 1.0])))?;
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let pa = wait!(red.readback())?.pixels;
    let pb = wait!(blue.readback())?.pixels;
    // Center pixel of the 8..56 fill rect, row-major.
    let center = 32 * 64 + 32;
    assert!(
        pa[center][0] > 0.9,
        "red surface red channel: {:?}",
        pa[center]
    );
    assert!(
        pb[center][2] > 0.9,
        "blue surface blue channel: {:?}",
        pb[center]
    );
    Ok(())
}
}

split_test! {
/// Reflecting a symmetric shape must preserve its fractional edge coverage.
fn reflected_shape_preserves_antialiasing() -> Result<(), Box<dyn std::error::Error>> {
    use cherenkov::kurbo::{Affine, Circle};

    let engine = wait!(Engine::<Gpu>::new(GpuConfig::default()))?;
    let normal = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    let reflected = wait!(engine.surface(Offscreen::new((32, 32), OffscreenFormat::LinearF16)))?;
    for (surface, transform) in [
        (&normal, Affine::IDENTITY),
        (&reflected, Affine::new([-1.0, 0.0, 0.0, 1.0, 32.0, 0.0])),
    ] {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|recorder| {
                recorder.transform(transform, |recorder| {
                    recorder.fill(Circle::new((16.0, 16.0), 9.25), WorkingColor::WHITE);
                });
            }));
        });
    }
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let normal = wait!(normal.readback())?;
    let reflected = wait!(reflected.readback())?;
    assert!(
        normal
            .pixels
            .iter()
            .any(|pixel| pixel[3] > 0.0 && pixel[3] < 1.0)
    );
    for (index, (normal, reflected)) in normal.pixels.iter().zip(&reflected.pixels).enumerate() {
        assert!(
            (normal[3] - reflected[3]).abs() < 0.001,
            "reflection changed edge coverage at pixel {index}: {} vs {}",
            normal[3],
            reflected[3]
        );
    }
    Ok(())
}
}
