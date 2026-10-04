//! Raster smoke tests: exact-area coverage, readback formats, fill rules,
//! transform and group isolation.

#![cfg(not(target_arch = "wasm32"))]
use cherenkov::kurbo::{Affine, BezPath, Line, Rect};
use cherenkov::{
    Draw, EvenOdd, Extend, Glyph, GlyphRun, Group, Interpolation, LinearGradient, Paint,
    RadialGradient, Shadow, SweepGradient, WorkingColor, kurbo::Stroke,
};
use cherenkov::{Engine, FrameTime, Offscreen, OffscreenFormat};
use cherenkov_cpu::{Raster, RasterConfig};

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

const RED: WorkingColor = WorkingColor::new([1., 0., 0., 1.]);

#[test]
fn a_half_edge_rect_has_exact_coverage() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()]
            .content(surface.record(|c| c.fill(Rect::new(8.5, 8.5, 40.5, 40.5), RED)));
    });
    engine.render(FrameTime::now()).expect("render");
    let rb = surface.readback().expect("readback");
    let at = |x: usize, y: usize| rb.pixels[y * 64 + x];
    assert_eq!(at(20, 20), [1.0, 0.0, 0.0, 1.0], "interior");
    let left = at(8, 20);
    assert!(
        (left[0] - 0.5).abs() < 1e-6 && (left[3] - 0.5).abs() < 1e-6,
        "left edge: {left:?}"
    );
    let right = at(40, 20);
    assert!(
        (right[0] - 0.5).abs() < 1e-6 && (right[3] - 0.5).abs() < 1e-6,
        "right edge: {right:?}"
    );
    // Fully transparent outside.
    assert_eq!(at(0, 0), [0.0; 4]);
}

#[test]
fn linear_f16_is_the_f16_rounding_of_f32() {
    let engine = engine();
    for format in [OffscreenFormat::LinearF32, OffscreenFormat::LinearF16] {
        let surface = engine
            .surface(Offscreen::new((64, 64), format))
            .expect("surface");
        surface.update(|tx| {
            tx[surface.root()]
                .content(surface.record(|c| c.fill(Rect::new(8.5, 8.5, 40.5, 40.5), RED)));
        });
        engine.render(FrameTime::now()).expect("render");
        let rb = surface.readback().expect("readback");
        let px = rb.pixels[20 * 64 + 8];
        match format {
            OffscreenFormat::LinearF32 => assert!((px[3] - 0.5).abs() < 1e-6),
            OffscreenFormat::LinearF16 => {
                let half = half::f16::from_f32(0.5).to_f32();
                assert_eq!(px[3], half);
            }
        }
    }
}

#[test]
fn even_odd_leaves_the_centre_of_concentric_squares_empty() {
    let mut path = BezPath::new();
    for (a, b) in [(4.0, 60.0), (16.0, 48.0)] {
        path.move_to((a, a));
        path.line_to((b, a));
        path.line_to((b, b));
        path.line_to((a, b));
        path.close_path();
    }
    let engine = engine();
    let centre = |even_odd: bool| {
        let surface = engine
            .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32))
            .expect("surface");
        let content = surface.record(|c| {
            if even_odd {
                c.fill(EvenOdd(path.clone()), RED);
            } else {
                c.fill(path.clone(), RED);
            }
        });
        surface.update(|tx| {
            tx[surface.root()].content(content);
        });
        engine.render(FrameTime::now()).expect("render");
        surface.readback().expect("readback").pixels[32 * 64 + 32]
    };
    assert_eq!(centre(true), [0.0; 4], "even-odd centre");
    assert_eq!(centre(false), [1.0, 0.0, 0.0, 1.0], "non-zero centre");
}

#[test]
fn a_rotated_rects_coverage_sums_to_its_area() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.transform(
                Affine::translate((32.0, 32.0)) * Affine::rotate(std::f64::consts::FRAC_PI_4),
                |c| c.fill(Rect::new(-10.0, -10.0, 10.0, 10.0), RED),
            );
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let rb = surface.readback().expect("readback");
    let area: f64 = rb.pixels.iter().map(|px| f64::from(px[3])).sum();
    let expected = 400.0; // 20 × 20.
    assert!(
        (area - expected).abs() < expected * 0.005,
        "rotated rect coverage {area} vs {expected}"
    );
}

#[test]
fn a_half_opacity_group_halves_the_alpha() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.group(Group::new().opacity(0.5), |c| {
                c.fill(Rect::new(8.0, 8.0, 40.0, 40.0), RED);
            });
        }));
    });
    engine.render(FrameTime::now()).expect("render");
    let rb = surface.readback().expect("readback");
    let px = rb.pixels[20 * 64 + 20];
    assert!((px[3] - 0.5).abs() < 1e-6, "group alpha: {px:?}");
    assert!((px[0] - 0.5).abs() < 1e-6, "premultiplied red: {px:?}");
}

/// Renders `body` into a `LinearF32` surface and returns pixels.
fn render_f32(
    engine: &Engine<Raster>,
    width: u32,
    height: u32,
    body: impl FnOnce(&mut cherenkov::Recorder),
) -> Vec<[f32; 4]> {
    let surface = engine
        .surface(Offscreen::new((width, height), OffscreenFormat::LinearF32))
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(body));
    });
    engine.render(FrameTime::now()).expect("render");
    surface.readback().expect("readback").pixels
}

#[test]
fn an_open_subpath_fills_as_if_closed() {
    let engine = engine();
    let alpha_sum = |closed: bool| {
        let mut path = BezPath::new();
        path.move_to((16.0, 8.0));
        path.line_to((48.0, 8.0));
        path.line_to((32.0, 56.0));
        if closed {
            path.close_path();
        }
        render_f32(&engine, 64, 64, |c| c.fill(path.clone(), RED))
            .iter()
            .map(|px| f64::from(px[3]))
            .sum::<f64>()
    };
    let (open, closed) = (alpha_sum(false), alpha_sum(true));
    assert!(
        (open - closed).abs() < 1e-9,
        "open {open} vs closed {closed}"
    );
    assert!(open > 100.0, "triangle area {open}");
}

#[test]
fn a_linear_gradient_pads_repeats_and_reflects() {
    let engine = engine();
    // Pad: red → blue over x ∈ [0, 64] on a 64×8 surface.
    let grad = LinearGradient::new((0.0, 0.0), (64.0, 0.0))
        .stop(0.0, RED)
        .stop(1.0, WorkingColor::new([0., 0., 1., 1.]));
    let px = render_f32(&engine, 64, 8, |c| {
        c.fill(Rect::new(0.0, 0.0, 64.0, 8.0), Paint::from(grad));
    });
    // x = 16.5 is t ≈ 0.258 → r ≈ 0.742, b ≈ 0.258.
    let p = px[4 * 64 + 16];
    assert!(
        (p[0] - 0.742).abs() < 1e-3 && (p[2] - 0.258).abs() < 1e-3,
        "t≈0.26: {p:?}"
    );
    // Beyond the end pads to the last stop.
    let grad_pad = LinearGradient::new((0.0, 0.0), (16.0, 0.0))
        .stop(0.0, RED)
        .stop(1.0, WorkingColor::new([0., 0., 1., 1.]))
        .extend(Extend::Pad);
    let px = render_f32(&engine, 64, 8, |c| {
        c.fill(Rect::new(0.0, 0.0, 64.0, 8.0), Paint::from(grad_pad));
    });
    let p = px[4 * 64 + 40];
    assert!(
        (p[0]).abs() < 1e-3 && (p[2] - 1.0).abs() < 1e-3,
        "pad end: {p:?}"
    );
    // Repeat: x = 24.5 → t = 8.5/16 ≈ 0.53; Reflect: x = 40.5 → t = 0.47.
    let mk = |extend| {
        LinearGradient::new((0.0, 0.0), (16.0, 0.0))
            .stop(0.0, RED)
            .stop(1.0, WorkingColor::new([0., 0., 1., 1.]))
            .extend(extend)
    };
    let px = render_f32(&engine, 64, 8, |c| {
        c.fill(
            Rect::new(0.0, 0.0, 64.0, 8.0),
            Paint::from(mk(Extend::Repeat)),
        );
    });
    let p = px[4 * 64 + 24];
    let t = 8.5 / 16.0;
    assert!(
        (p[0] - (1.0 - t)).abs() < 1e-2 && (p[2] - t).abs() < 1e-2,
        "repeat: {p:?}"
    );
    let px = render_f32(&engine, 64, 8, |c| {
        c.fill(
            Rect::new(0.0, 0.0, 64.0, 8.0),
            Paint::from(mk(Extend::Reflect)),
        );
    });
    let p = px[4 * 64 + 40];
    let t = 8.5 / 16.0;
    assert!(
        (p[0] - (1.0 - t)).abs() < 1e-2 && (p[2] - t).abs() < 1e-2,
        "reflect: {p:?}"
    );
}

#[test]
fn a_radial_gradient_interpolates_from_the_centre() {
    let engine = engine();
    let grad = RadialGradient::new((32.0, 32.0), 16.0)
        .stop(0.0, RED)
        .stop(1.0, WorkingColor::new([0., 0., 1., 1.]));
    let px = render_f32(&engine, 64, 64, |c| {
        c.fill(Rect::new(0.0, 0.0, 64.0, 64.0), Paint::from(grad));
    });
    // Pixel (40, 32) is at distance 8.5 → t ≈ 0.53.
    let p = px[32 * 64 + 40];
    let t = 8.5 / 16.0;
    assert!(
        (p[0] - (1.0 - t)).abs() < 2e-2 && (p[2] - t).abs() < 2e-2,
        "radial t≈{t}: {p:?}"
    );
}

#[test]
fn srgb_encoded_interpolation_midpoint_is_not_linear_half() {
    let engine = engine();
    let grad = LinearGradient::new((0.0, 0.0), (64.0, 0.0))
        .stop(0.0, WorkingColor::new([0., 0., 0., 1.]))
        .stop(1.0, WorkingColor::new([1., 1., 1., 1.]))
        .interpolation(Interpolation::SrgbEncoded);
    let px = render_f32(&engine, 64, 8, |c| {
        c.fill(Rect::new(0.0, 0.0, 64.0, 8.0), Paint::from(grad));
    });
    let p = px[4 * 64 + 32];
    // Encoded midpoint ≈ 0.5 → decoded linear ≈ 0.2140 (in sRGB ≈ P3).
    assert!((p[0] - 0.2140).abs() < 1e-2, "srgb midpoint: {p:?}");
}

#[test]
fn a_stroked_lines_alpha_sum_is_width_times_length() {
    let engine = engine();
    let px = render_f32(&engine, 64, 64, |c| {
        c.stroke(
            Line::new((8.0, 32.0), (56.0, 32.0)),
            Stroke::new(4.0).with_caps(kurbo::Cap::Butt),
            RED,
        );
    });
    let area: f64 = px.iter().map(|p| f64::from(p[3])).sum();
    let expected = 4.0 * 48.0;
    assert!(
        (area - expected).abs() < expected * 0.01,
        "stroke area {area} vs {expected}"
    );
}

#[test]
fn a_zero_sigma_shadow_covers_the_rect() {
    let engine = engine();
    let px = render_f32(&engine, 64, 64, |c| {
        c.shadow(
            Rect::new(22.0, 22.0, 42.0, 42.0),
            Shadow::new(0.0, WorkingColor::new([0., 0., 0., 1.])),
        );
    });
    let area: f64 = px.iter().map(|p| f64::from(p[3])).sum();
    // sigma_eff = sqrt(1/6) slightly leaks past 400 px²; within 2%.
    assert!((area - 400.0).abs() < 8.0, "shadow area {area}");
    let centre = px[32 * 64 + 32];
    assert!((centre[3] - 1.0).abs() < 1e-2, "centre alpha: {centre:?}");
}

#[test]
fn a_glyph_run_renders_and_the_second_frame_hits_the_cache() {
    let engine = engine();
    let data = std::fs::read("../scenes/fonts/NotoSans.ttf").expect("test font");
    let font = engine
        .font(cherenkov::FontSource::bytes(data))
        .expect("font");
    let run = GlyphRun {
        font: font.id(),
        size: 32.0,
        coords: Vec::new().into(),
        glyphs: vec![Glyph {
            id: 36, // 'A' in most Latin fonts
            x: 8.0,
            y: 40.0,
            transform: None,
        }]
        .into(),
        style: cherenkov::GlyphStyle::Fill,
    };
    let px = render_f32(&engine, 64, 64, |c| c.glyphs(run.clone(), RED));
    let area: f64 = px.iter().map(|p| f64::from(p[3])).sum();
    assert!(area > 10.0, "glyph coverage {area}");
    let cached = engine.memory().cpu;
    assert!(cached.0 > 0, "glyph cache populated");
    // Re-record the same run and render again: the cache must hit.
    render_f32(&engine, 64, 64, |c| c.glyphs(run.clone(), RED));
    assert_eq!(engine.memory().cpu.0, cached.0, "cache hit");
}

#[test]
fn sweep_renders_and_remaining_unsupported_features_report_their_names() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32))
        .expect("surface");
    let sweep = SweepGradient::new((32.0, 32.0), 0.0, std::f64::consts::TAU)
        .stop(0.0, RED)
        .stop(1.0, WorkingColor::new([0., 0., 1., 1.]));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| {
            c.fill(Rect::new(0.0, 0.0, 64.0, 64.0), Paint::from(sweep));
        }));
    });
    engine.render(FrameTime::now()).expect("sweep supported");
    let pixel = surface.readback().expect("sweep pixels").pixels[32 * 64 + 48];
    let angle = 0.5_f64.atan2(16.5) / std::f64::consts::TAU;
    assert!((f64::from(pixel[0]) - (1.0 - angle)).abs() < 1e-6);
    assert!((f64::from(pixel[2]) - angle).abs() < 1e-6);
    assert_eq!(pixel[3].to_bits(), 1.0_f32.to_bits());
    // A rotated glyph renders as outline coverage.
    let engine2 = Engine::<Raster>::new(RasterConfig::default()).expect("engine");
    let data = std::fs::read("../scenes/fonts/NotoSans.ttf").expect("test font");
    let font = engine2
        .font(cherenkov::FontSource::bytes(data))
        .expect("font");
    let run = GlyphRun {
        font: font.id(),
        size: 32.0,
        coords: Vec::new().into(),
        glyphs: vec![Glyph {
            id: 36,
            x: 8.0,
            y: 40.0,
            transform: Some(Affine::rotate(0.5)),
        }]
        .into(),
        style: cherenkov::GlyphStyle::Fill,
    };
    let surface2 = engine2
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32))
        .expect("surface");
    surface2.update(|tx| {
        tx[surface2.root()].content(surface2.record(|c| c.glyphs(run.clone(), RED)));
    });
    engine2
        .render(FrameTime::now())
        .expect("glyph transform renders as outline coverage");
    let area: f64 = surface2
        .readback()
        .expect("transformed pixels")
        .pixels
        .iter()
        .map(|p| f64::from(p[3]))
        .sum();
    assert!(area > 10.0, "transformed glyph coverage {area}");
}
