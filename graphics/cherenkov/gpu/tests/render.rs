//! Integration tests for the GPU backend. They skip when no GPU adapter is
//! available, which CI may not have.

use cherenkov::kurbo::{BezPath, Rect};
use cherenkov::{__engine_fn as split_fn, __engine_test as split_test, __engine_wait as wait};
use cherenkov::{Draw, WorkingColor};
use cherenkov::{Engine, EngineError, Next, Offscreen, OffscreenFormat, RenderError};
use cherenkov_gpu::{Gpu, GpuConfig};

split_fn! {
fn render_card(
    engine: &Engine<Gpu>,
    alpha: f32,
) -> Result<cherenkov::Readback, Box<dyn std::error::Error>> {
    let surface = wait!(engine.surface(Offscreen::new((128, 128), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            let card = Rect::new(24., 24., 104., 104.);
            c.shadow(
                card,
                cherenkov::Shadow::new(6.0, WorkingColor::new([0., 0., 0., 1.]))
                    .offset((2., 3.)),
            );
            c.fill(card, WorkingColor::new([0.9, 0.3, 0.1, alpha]));
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    Ok(wait!(surface.readback())?)
}
}

split_fn! {
/// An engine, or `None` when no adapter exists.
fn engine() -> Option<Engine<Gpu>> {
    match wait!(Engine::new(GpuConfig::default())) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(e) => panic!("engine init failed: {e}"),
    }
}
}

split_test! {
fn a_red_rect_renders_and_reads_back() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(8., 8., 56., 56.),
                WorkingColor::new([1., 0., 0., 1.]),
            );
        }));
    });
    let next = wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(next, Next::Idle);
    let readback = wait!(surface.readback())?;
    let px = |x: u32, y: u32| readback.pixels[(y * readback.width + x) as usize];
    let [r, g, b, a] = px(32, 32);
    assert!(
        (r - 1.0).abs() < 1e-2 && g.abs() < 1e-2 && b.abs() < 1e-2 && (a - 1.0).abs() < 1e-2,
        "center pixel: {r} {g} {b} {a}"
    );
    assert_eq!(px(2, 2), [0.0; 4], "corner pixel must be the clear colour");
    Ok(())
}
}

split_test! {
/// Pushing a layer under its own subtree — the surface root under a
/// descendant included — would close a cycle the lowering recursion
/// cannot escape; the commit must reject the op.
fn a_cyclic_layer_tree_is_rejected() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16)))?;
    let a = surface.layer();
    let b = surface.layer();
    surface.update(|tx| {
        tx[surface.root()].push(&a);
        tx[&a].push(&b);
        tx[&b].push(&a);
    });
    // The shared tree fails before lowering can recurse into the cycle.
    assert!(matches!(
        wait!(engine.render(cherenkov::FrameTime::now())),
        Err(RenderError::Thread)
    ));
    Ok(())
}
}

split_test! {
fn a_path_fill_renders() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let mut path = BezPath::new();
    path.move_to((4., 4.));
    path.curve_to((20., 60.), (44., 60.), (60., 4.));
    path.close_path();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(path, WorkingColor::new([1., 0., 0., 1.]));
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    let [r, ..] = readback.pixels[(30 * readback.width + 30) as usize];
    assert!(r > 0.5, "interior pixel: {r}");
    Ok(())
}
}

split_test! {
fn a_path_shadow_renders() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let mut path = BezPath::new();
    path.move_to((4., 4.));
    path.curve_to((20., 60.), (44., 60.), (60., 4.));
    path.close_path();
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.shadow(
                path.clone(),
                cherenkov::Shadow::new(4.0, WorkingColor::new([0., 0., 0., 1.])),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    let mut scene = cherenkov_scene::Scene::new(
        64,
        64,
        cherenkov_scene::Color::new(cherenkov_scene::ColorSpace::LinearP3, [0.; 4]),
    );
    scene
        .root
        .items
        .push(cherenkov_scene::Item::Draw(cherenkov_scene::Draw::Shadow {
            shape: cherenkov_scene::Shape::Path { path },
            blur_sigma: 4.0,
            offset: [0.0; 2],
            color: cherenkov_scene::Color::new(
                cherenkov_scene::ColorSpace::LinearP3,
                [0., 0., 0., 1.],
            ),
        }));
    let expected =
        cherenkov_oracle::Renderer::new(64, 64).render(&scene, std::path::Path::new("."))?;
    // Compare the whole premultiplied image, including the blur fringe and
    // transparent exterior, against independent f64 coverage/convolution.
    for (index, (actual, expected)) in readback.pixels.iter().zip(expected.pixels).enumerate() {
        for (actual, expected) in actual.iter().zip(expected) {
            assert!(
                (actual - expected).abs() < 0.005,
                "pixel {index}: {actual} != oracle {expected}"
            );
        }
    }
    Ok(())
}
}

split_test! {
/// Two dirty surfaces sharing one frame: the second surface lowers far
/// more instances than the initial instance buffer holds, forcing a grow
/// that must preserve the first surface's upload.
fn an_earlier_surfaces_uploads_survive_a_shared_buffer_grow()
-> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let small = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    let big = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    small.update(|tx| {
        tx[small.root()].content(small.record(|c| {
            c.fill(
                Rect::new(8., 8., 56., 56.),
                WorkingColor::new([1., 0., 0., 1.]),
            );
        }));
    });
    big.update(|tx| {
        tx[big.root()].content(big.record(|c| {
            // 3000 4×4 green rects in a grid — far past the 16-instance
            // initial buffer.
            for i in 0..3000u32 {
                let x = f64::from(i % 55) * 1.0;
                let y = f64::from(i / 55) * 1.0;
                if y > 60.0 {
                    break;
                }
                c.fill(
                    Rect::new(x, y, x + 0.5, y + 0.5),
                    WorkingColor::new([0., 1., 0., 1.]),
                );
            }
        }));
    });
    let next = wait!(engine.render(cherenkov::FrameTime::now()))?;
    assert_eq!(next, Next::Idle);
    let small_rb = wait!(small.readback())?;
    let [r, g, b, a] = small_rb.pixels[(32 * small_rb.width + 32) as usize];
    assert!(
        (r - 1.0).abs() < 1e-2 && g.abs() < 1e-2 && b.abs() < 1e-2 && (a - 1.0).abs() < 1e-2,
        "first surface's pixel must still be red: {r} {g} {b} {a}"
    );
    let big_rb = wait!(big.readback())?;
    let [r, g, b, a] = big_rb.pixels[(4 * big_rb.width + 4) as usize];
    // 0.5-wide rects cover the pixel partially; green is what matters.
    assert!(
        g > 0.1 && r < 0.1,
        "second surface must render green: {r} {g} {b} {a}"
    );
    Ok(())
}
}

split_test! {
/// Sixty timed frames of a text-and-fill scene that grows the atlas and the
/// shared buffers: every frame must finish within the wait bound, and when
/// the adapter samples timestamps the whole-frame GPU time must come back
/// alongside the per-pass ones (pass-boundary timestamps only).
fn many_timed_frames_complete_with_whole_frame_gpu_time() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(engine) = wait!(timed_engine(wgpu::Backends::all())) else {
        return Ok(());
    };
    wait!(many_timed_frames(&engine))
}
}

split_test! {
/// The Metal-only counterpart: macOS always has a Metal adapter, so this
/// fails rather than skips when it is missing. Apple GPUs sample only at
/// stage boundaries, which the pass-boundary timestamps rely on.
#[cfg(target_os = "macos")]
fn metal_times_many_frames_at_pass_boundaries() -> Result<(), Box<dyn std::error::Error>> {
    let engine = wait!(timed_engine(wgpu::Backends::METAL)).expect("a Metal adapter");
    assert_eq!(engine.info().backend, "Metal", "{:?}", engine.info());
    wait!(many_timed_frames(&engine))
}
}

split_fn! {
/// A timestamping engine on `backends` with a short wait bound, or `None`
/// when they have no adapter.
fn timed_engine(backends: wgpu::Backends) -> Option<Engine<Gpu>> {
    match wait!(Engine::new(GpuConfig {
        backends,
        timestamps: true,
        wait_timeout: std::time::Duration::from_secs(20),
        ..GpuConfig::default()
    })) {
        Ok(engine) => Some(engine),
        Err(EngineError::Backend(_)) => None,
        Err(e) => panic!("engine init failed: {e}"),
    }
}
}

split_fn! {
fn many_timed_frames(engine: &Engine<Gpu>) -> Result<(), Box<dyn std::error::Error>> {
    let timed = engine.info().timestamps != cherenkov_gpu::TimestampSupport::Unsupported;
    let font = engine.font(cherenkov::FontSource::bytes(std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/NotoSans.ttf"
    ))?))?;
    let surface = wait!(engine.surface(Offscreen::new((256, 256), OffscreenFormat::LinearF16)))?;
    let mut submitted = Vec::new();
    for frame in 0..60u32 {
        // A different glyph size each frame keeps rasterising new atlas
        // entries so the atlas grows and eventually clears.
        let run = cherenkov::GlyphRun {
            font: font.id(),
            size: 12.0 + f32::from(u16::try_from(frame)?),
            coords: Vec::new().into(),
            glyphs: (0..40u16)
                .map(|i| cherenkov::Glyph {
                    id: 1 + (u32::from(i) + frame) % 60,
                    x: f32::from(i % 10) * 24.0,
                    y: f32::from(i / 10).mul_add(50.0, 40.0),
                    transform: None,
                })
                .collect::<Vec<_>>()
                .into(),
            style: cherenkov::GlyphStyle::Fill,
        };
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                for i in 0..2000u32 {
                    let x = f64::from(i % 50) * 5.0;
                    let y = f64::from(i / 50) * 6.0;
                    c.fill(
                        Rect::new(x, y, x + 4.0, y + 4.0),
                        WorkingColor::new([0., 1., 0., 1.]),
                    );
                }
                c.glyphs(run.clone(), WorkingColor::WHITE);
            }));
        });
        let next = wait!(engine.render(cherenkov::FrameTime::now()))?;
        assert_eq!(next, Next::Idle, "frame {frame}");
        let stats = engine.stats();
        assert!(stats.passes > 0, "frame {frame} drew nothing: {stats:?}");
        submitted.push((stats.frame.expect("a drawing render submits"), stats.passes));
    }
    let timings = wait!(engine.finish_timings())?;
    assert!(
        wait!(engine.finish_timings())?.is_empty(),
        "timings are consumed once"
    );
    assert_eq!(wait!(engine.render(cherenkov::FrameTime::now()))?, Next::Idle);
    let idle = engine.stats();
    assert!(idle.frame.is_none(), "{idle:?}");
    if timed {
        // Every submitted frame's timing arrives exactly once, in order,
        // tagged with its frame, and spans that frame's passes.
        assert_eq!(
            timings.iter().map(|t| t.frame).collect::<Vec<_>>(),
            submitted.iter().map(|(f, _)| *f).collect::<Vec<_>>(),
            "every frame is timed once, in order"
        );
        for (timing, (_, passes)) in timings.iter().zip(&submitted) {
            assert_eq!(timing.passes.len(), *passes as usize, "{timing:?}");
            let gpu = timing.gpu_seconds.expect("the frame's timestamps increase");
            let spanned: f64 = timing
                .passes
                .iter()
                .map(|p| p.gpu_seconds.expect("the pass's timestamps increase"))
                .sum();
            assert!(
                gpu > 0.0 && gpu >= spanned * 0.99,
                "{:?}: whole frame {gpu}s must span its passes {spanned}s",
                timing.frame
            );
        }
    } else {
        assert!(timings.is_empty());
    }
    let readback = wait!(surface.readback())?;
    let [r, g, ..] = readback.pixels[(2 * readback.width + 2) as usize];
    assert!(g > 0.5 && r < 0.1, "fills must still render: {r} {g}");
    Ok(())
}
}

split_test! {
/// A `Shadow` immediately followed by an opaque solid fill of the same
/// shape lowers to up-to-four border quads: the covered interior is
/// skipped. An opaque card's result must be pixel-identical outside the
/// card and within fill-alpha error inside, vs the same card whose fill
/// is alpha 0.999 (which disables the split).
fn a_shadow_under_an_opaque_fill_loses_only_its_interior() -> Result<(), Box<dyn std::error::Error>>
{
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let split = wait!(render_card(&engine, 1.0))?;
    let whole = wait!(render_card(&engine, 0.999))?;
    for y in 0..whole.height {
        for x in 0..whole.width {
            let i = (y * whole.width + x) as usize;
            let inside = (24.0..=104.0).contains(&(f64::from(x) + 0.5))
                && (24.0..=104.0).contains(&(f64::from(y) + 0.5));
            // Strips vs one quad interpolate `local` over different
            // corners, so coverage can round one f16 ulp (~1e-3 at 1.0)
            // either way; 2e-3 bounds that while still catching a
            // missed or doubled rasterization region.
            let tol = if inside { 1e-2 } else { 2e-3 };
            for c in 0..4 {
                let d = (split.pixels[i][c] - whole.pixels[i][c]).abs();
                assert!(
                    d <= tol,
                    "pixel ({x},{y}) ch {c}: split {} vs whole {}",
                    split.pixels[i][c],
                    whole.pixels[i][c]
                );
            }
        }
    }
    Ok(())
}
}

split_test! {
/// A large axis-aligned box fill lowers to one `KIND_SPAN` interior plus
/// border quads: sampled pixels must equal the analytic gradient, and an
/// edge pixel must still show partial coverage.
fn a_large_fill_spans_its_interior() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let surface = wait!(engine.surface(Offscreen::new((320, 320), OffscreenFormat::LinearF16)))?;
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(
                Rect::new(10.5, 10.5, 309.5, 309.5),
                cherenkov::LinearGradient::new((10., 0.), (310., 0.))
                    .stop(0.0, WorkingColor::new([1., 0., 0., 1.]))
                    .stop(1.0, WorkingColor::new([0., 0., 1., 1.])),
            );
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let readback = wait!(surface.readback())?;
    let px = |x: u32, y: u32| readback.pixels[(y * readback.width + x) as usize];
    for (px_x, px_y) in [
        (40u32, 160u32),
        (160, 160),
        (280, 160),
        (160, 40),
        (160, 280),
    ] {
        let [red, green, blue, alpha] = px(px_x, px_y);
        let frac = (f64::from(px_x) - 10.0 + 0.5) / 300.0;
        assert!(
            (f64::from(red) - (1.0 - frac)).abs() < 1.0 / 255.0
                && f64::from(green) < 1.0 / 255.0
                && (f64::from(blue) - frac).abs() < 1.0 / 255.0
                && (f64::from(alpha) - 1.0).abs() < 1.0 / 255.0,
            "pixel ({px_x},{px_y}): {red} {green} {blue} {alpha}, expected frac {frac}"
        );
    }
    // Pixel x == 10 has its centre on the rect's left edge (10.5): it is
    // half-covered by antialiasing.
    let [.., edge_a] = px(10, 160);
    assert!(
        edge_a > 0.05 && edge_a < 0.95,
        "edge pixel must be partially covered: {edge_a}"
    );
    Ok(())
}
}

split_test! {
/// Per-variant pipelines: a frame with shadow strips, a solid fill, a
/// clipped gradient fill, and a glyph run must emit ranges for at least
/// the Simple/Shadow/Full variants — while the pixels stay correct.
fn variants_split_ranges_but_not_pixels() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let font = engine.font(cherenkov::FontSource::bytes(std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../scenes/fonts/NotoSans.ttf"
    ))?))?;
    let surface = wait!(engine.surface(Offscreen::new((128, 96), OffscreenFormat::LinearF16)))?;
    let run = cherenkov::GlyphRun {
        font: font.id(),
        size: 24.0,
        coords: Vec::new().into(),
        glyphs: vec![cherenkov::Glyph {
            id: 1,
            x: 32.0,
            y: 88.0,
            transform: None,
        }]
        .into(),
        style: cherenkov::GlyphStyle::Fill,
    };
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            let card = Rect::new(8., 8., 56., 56.);
            c.shadow(
                card,
                cherenkov::Shadow::new(3.0, WorkingColor::new([0., 0., 0., 1.])).offset((2., 2.)),
            );
            c.fill(card, WorkingColor::new([1., 0., 0., 1.]));
            c.clip(Rect::new(80., 8., 120., 56.), |c| {
                c.fill(
                    Rect::new(80., 8., 128., 56.),
                    cherenkov::LinearGradient::new((80., 0.), (128., 0.))
                        .stop(0.0, WorkingColor::new([1., 0., 0., 1.]))
                        .stop(1.0, WorkingColor::new([0., 0., 1., 1.])),
                );
            });
            c.glyphs(run, WorkingColor::WHITE);
        }));
    });
    wait!(engine.render(cherenkov::FrameTime::now()))?;
    let stats = engine.stats();
    assert!(
        stats.pipeline_switches >= 2 && stats.draws >= 3,
        "expected variant-split ranges: {stats:?}"
    );
    let readback = wait!(surface.readback())?;
    let px = |x: u32, y: u32| readback.pixels[(y * readback.width + x) as usize];
    assert_eq!(
        px(32, 32),
        [1.0, 0.0, 0.0, 1.0],
        "solid fill centre must be exactly opaque red"
    );
    let [r, g, b, a] = px(100, 30);
    let t = (100.5 - 80.0) / 48.0;
    assert!(
        (f64::from(r) - (1.0 - t)).abs() < 1.0 / 255.0
            && f64::from(g) < 1.0 / 255.0
            && (f64::from(b) - t).abs() < 1.0 / 255.0
            && (f64::from(a) - 1.0).abs() < 1.0 / 255.0,
        "gradient under clip at (100,30): {r} {g} {b} {a}, t {t}"
    );
    assert_eq!(
        px(125, 80),
        [0.0; 4],
        "inside the fill rect but outside the clip: nothing"
    );
    Ok(())
}
}

split_test! {
fn retained_painter_commands_follow_content_revisions() -> Result<(), Box<dyn std::error::Error>> {
    let Some(engine) = wait!(engine()) else {
        return Ok(());
    };
    let actual = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
    // Same command layout with new instance data, buffer growth, fewer
    // commands, and an empty pass must all agree with a fresh recording.
    for (count, color) in [(1, [1., 0., 0., 1.]), (1, [0., 1., 0., 1.]),
        (96, [0., 0., 1., 0.5]), (2, [1., 1., 0., 1.]), (0, [0.; 4]), (3, [1.; 4])] {
        let record = |c: &mut cherenkov::Recorder| {
            for index in 0..count {
                let x = f64::from(index % 4).mul_add(12.0, 8.0);
                let y = f64::from((index / 4) % 4).mul_add(12.0, 8.0);
                let rect = Rect::new(x, y, x + 6.0, y + 6.0);
                c.shadow(rect, cherenkov::Shadow::new(1.0, WorkingColor::new([0., 0., 0., 0.5])));
                c.fill(rect, WorkingColor::new(color));
            }
        };
        actual.update(|tx| { tx[actual.root()].content(actual.record(record)); });
        wait!(engine.render(cherenkov::FrameTime::now()))?;
        let pixels = wait!(actual.readback())?.pixels;
        let fresh = wait!(engine.surface(Offscreen::new((64, 64), OffscreenFormat::LinearF16)))?;
        fresh.update(|tx| { tx[fresh.root()].content(fresh.record(record)); });
        wait!(engine.render(cherenkov::FrameTime::now()))?;
        assert_eq!(pixels, wait!(fresh.readback())?.pixels, "painter revision with {count} cards");
    }
    Ok(())
}
}
