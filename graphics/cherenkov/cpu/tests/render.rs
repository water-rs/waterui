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
use cherenkov_oracle::F32Image;
use cherenkov_oracle::metrics::{assert_close, assert_images_match};

fn engine() -> Engine<Raster> {
    Engine::<Raster>::new(RasterConfig::default()).expect("engine")
}

const RED: WorkingColor = WorkingColor::new([1., 0., 0., 1.]);

#[test]
fn retained_outputs_skip_unchanged_bands_after_motion_and_clear() {
    compare_retained_outputs(false);
}

#[test]
fn retained_outputs_expand_damage_through_nested_filters() {
    compare_retained_outputs(true);
}

fn compare_retained_outputs(filtered: bool) {
    for format in [OffscreenFormat::LinearF32, OffscreenFormat::LinearF16] {
        let engine = engine();
        let size = (128, 1024);
        let surface = engine.surface(Offscreen::new(size, format), || {}).unwrap();
        let moving = surface.layer();
        let outer = surface.layer();
        let inner = surface.layer();
        let blur = filtered.then(|| engine.filter(filtrate::filters::GaussianBlur::new(2.0f32)));
        let background = WorkingColor::new([0.1, 0.3, 0.2, 0.5]);
        let foreground = WorkingColor::new([3.0, 0.2, 0.8, 0.6]);
        surface.update(|tx| {
            tx[surface.root()].push(&outer).content(surface.record(|r| {
                r.fill(Rect::new(0.0, 0.0, 128.0, 1024.0), background);
            }));
            tx[&outer].push(&inner);
            tx[&inner].push(&moving);
            if let Some(blur) = &blur {
                tx[&outer].filter(blur.id());
                tx[&inner].filter(blur.id());
            }
        });
        for (step, position) in [
            Some((20.0, 32.0)),
            Some((80.0, 448.0)),
            None,
            Some((20.0, 32.0)),
        ]
        .into_iter()
        .enumerate()
        {
            surface.update(|tx| {
                if let Some(position) = position {
                    tx[&moving]
                        .transform(Affine::translate(position))
                        .content(surface.record(|r| {
                            r.fill(Rect::new(0.0, 0.0, 8.0, 8.0), foreground);
                        }));
                } else {
                    tx[&moving].clear_content();
                }
            });
            engine.render(FrameTime::now()).unwrap();
            let passes = engine.stats().passes;
            if step == 0 {
                assert_eq!(passes, 64);
            } else {
                assert!(
                    passes > 0 && passes * if filtered { 4 } else { 20 } < 64,
                    "frame {step}: {passes} bands"
                );
            }
            let actual = surface.readback().unwrap();
            let reference = engine.surface(Offscreen::new(size, format), || {}).unwrap();
            reference.update(|tx| {
                tx[reference.root()].content(reference.record(|r| {
                    r.fill(Rect::new(0.0, 0.0, 128.0, 1024.0), background);
                    if let Some((x, y)) = position {
                        let draw = |r: &mut cherenkov::Recorder| {
                            r.fill(Rect::new(x, y, x + 8.0, y + 8.0), foreground);
                        };
                        if let Some(blur) = &blur {
                            r.group(Group::new().filter(blur.id()), |r| {
                                r.group(Group::new().filter(blur.id()), draw);
                            });
                        } else {
                            draw(r);
                        }
                    }
                }));
            });
            engine.render(FrameTime::now()).unwrap();
            let expected = reference.readback().unwrap();
            let actual = F32Image {
                width: size.0,
                height: size.1,
                pixels: actual.pixels,
            };
            let expected = F32Image {
                width: size.0,
                height: size.1,
                pixels: expected.pixels,
            };
            assert_images_match(&expected, &actual);
            let path = std::env::temp_dir().join(format!(
                "cherenkov-cpu-damage-{filtered}-{format:?}-{step}.png"
            ));
            actual.write_png(&path).unwrap();
        }
    }
}

#[test]
fn retained_command_motion_preserves_background_and_layer_changes() {
    for format in [OffscreenFormat::LinearF32, OffscreenFormat::LinearF16] {
        let engine = engine();
        let size = (128, 1024);
        let surface = engine.surface(Offscreen::new(size, format), || {}).unwrap();
        let shape = nami::Binding::container(Rect::new(20.0, 32.0, 28.0, 40.0));
        let background = WorkingColor::new([0.1, 0.3, 0.2, 0.5]);
        let foreground = WorkingColor::new([3.0, 0.2, 0.8, 0.6]);
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|r| {
                r.fill(Rect::new(0.0, 0.0, 128.0, 1024.0), background);
                r.fill(shape.clone(), foreground);
            }));
        });
        for (step, (rect, opacity, empty)) in [
            (Rect::new(20.0, 32.0, 28.0, 40.0), 1.0_f32, false),
            (Rect::new(80.0, 448.0, 88.0, 456.0), 1.0_f32, false),
            (Rect::ZERO, 1.0_f32, false),
            (Rect::new(20.0, 32.0, 28.0, 40.0), 0.5_f32, false),
            (Rect::ZERO, 0.5_f32, true),
        ]
        .into_iter()
        .enumerate()
        {
            if step > 0 && !empty {
                shape.set(rect);
            }
            if step == 3 {
                surface.update(|tx| {
                    tx[surface.root()].opacity(opacity);
                });
            }
            if empty {
                surface.update(|tx| {
                    tx[surface.root()].content(surface.record(|_| {}));
                });
            }
            engine.render(FrameTime::now()).unwrap();
            let passes = engine.stats().passes;
            if step == 1 || step == 2 {
                assert!(
                    passes > 0 && passes < 4,
                    "command update repainted {passes} bands"
                );
            } else {
                assert_eq!(passes, 64, "whole-layer change {step}");
            }
            let actual = surface.readback().unwrap();
            let reference = engine.surface(Offscreen::new(size, format), || {}).unwrap();
            reference.update(|tx| {
                tx[reference.root()]
                    .opacity(opacity)
                    .content(reference.record(|r| {
                        if !empty {
                            r.fill(Rect::new(0.0, 0.0, 128.0, 1024.0), background);
                            r.fill(rect, foreground);
                        }
                    }));
            });
            engine.render(FrameTime::now()).unwrap();
            let expected = reference.readback().unwrap();
            let actual = F32Image {
                width: size.0,
                height: size.1,
                pixels: actual.pixels,
            };
            let expected = F32Image {
                width: size.0,
                height: size.1,
                pixels: expected.pixels,
            };
            assert_images_match(&expected, &actual);
            actual
                .write_png(&std::env::temp_dir().join(format!(
                    "cherenkov-cpu-command-damage-{format:?}-{step}.png"
                )))
                .unwrap();
        }
    }
}

#[test]
fn a_half_edge_rect_has_exact_coverage() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    surface.update(|tx| {
        tx[surface.root()]
            .content(surface.record(|c| c.fill(Rect::new(8.5, 8.5, 40.5, 40.5), RED)));
    });
    engine.render(FrameTime::now()).expect("render");
    let rb = surface.readback().expect("readback");
    let at = |x: usize, y: usize| rb.pixels[y * 64 + x];
    // Full coverage of an opaque fill writes the paint unchanged.
    assert_close(&at(20, 20), &[1.0, 0.0, 0.0, 1.0]);
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
    // No coverage leaves the cleared zero.
    assert_close(&at(0, 0), &[0.0; 4]);
}

#[test]
fn linear_f16_is_the_f16_rounding_of_f32() {
    let engine = engine();
    for format in [OffscreenFormat::LinearF32, OffscreenFormat::LinearF16] {
        let surface = engine
            .surface(Offscreen::new((64, 64), format), || {})
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
                // The F16 target stores the f16 rounding the reference computes the same way.
                assert_eq!(px[3].to_bits(), half.to_bits());
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
            .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
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
    // The even-odd hole has zero coverage: the cleared zero.
    assert_close(&centre(true), &[0.0; 4]);
    // Full non-zero coverage writes the opaque paint unchanged.
    assert_close(&centre(false), &[1.0, 0.0, 0.0, 1.0]);
}

#[test]
fn a_rotated_rects_coverage_sums_to_its_area() {
    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
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
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
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
        .surface(
            Offscreen::new((width, height), OffscreenFormat::LinearF32),
            || {},
        )
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
fn identical_radial_circles_cannot_repeat_or_reflect() {
    for extend in [Extend::Repeat, Extend::Reflect] {
        let engine = engine();
        let grad = RadialGradient {
            extend,
            ..RadialGradient::two_point((32.0, 32.0), 16.0, (32.0, 32.0), 16.0)
                .stop(0.0, RED)
                .stop(1.0, RED)
        };
        let surface = engine
            .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
            .expect("surface");
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|c| {
                c.fill(Rect::new(0.0, 0.0, 64.0, 64.0), Paint::from(grad.clone()));
            }));
        });
        let Err(err) = engine.render(FrameTime::now()) else {
            panic!("identical circles rendered under {extend:?}");
        };
        assert!(
            matches!(err, cherenkov::RenderError::IdenticalRadialCircles(e) if e == extend),
            "{extend:?}: {err}"
        );
    }
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
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
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
    assert_close(&[pixel[3]], &[1.0]);
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
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
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

/// A static scene records its content again every frame, and the new
/// lowering takes the swept geometry of the realization it replaces. The
/// result must be bit-identical to lowering the same content in a fresh
/// engine, including when only the paint changed between the records.
#[test]
fn re_recorded_overlapping_geometry_renders_like_a_fresh_lowering() {
    // A five-point star filled non-zero overlaps itself, and a thick
    // zigzag stroke overlaps at its joins: both are swept.
    let mut star = BezPath::new();
    for k in 0..5 {
        let angle = f64::from(k).mul_add(0.8, -0.5) * std::f64::consts::PI;
        let point = (
            28.0f64.mul_add(angle.cos(), 32.0),
            28.0f64.mul_add(angle.sin(), 32.0),
        );
        if k == 0 {
            star.move_to(point);
        } else {
            star.line_to(point);
        }
    }
    star.close_path();
    let mut zigzag = BezPath::new();
    zigzag.move_to((6.0, 50.0));
    zigzag.line_to((30.0, 10.0));
    zigzag.line_to((34.0, 58.0));
    zigzag.line_to((58.0, 12.0));
    let draw = |color: WorkingColor| {
        let (star, zigzag) = (star.clone(), zigzag.clone());
        move |c: &mut cherenkov::Recorder| {
            c.fill(star, color);
            c.stroke(zigzag, Stroke::new(9.0), color);
        }
    };
    let first = WorkingColor::new([0.0, 0.0, 1.0, 0.5]);
    let second = WorkingColor::new([1.0, 0.0, 0.0, 0.5]);

    let engine = engine();
    let surface = engine
        .surface(Offscreen::new((64, 64), OffscreenFormat::LinearF32), || {})
        .expect("surface");
    let mut frames = Vec::new();
    for color in [first, first, second] {
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(draw(color)));
        });
        engine.render(FrameTime::now()).expect("render");
        frames.push(surface.readback().expect("readback").pixels);
    }
    for (frame, color) in frames.iter().zip([first, first, second]) {
        let fresh = render_f32(&self::engine(), 64, 64, draw(color));
        assert_images_match(
            &F32Image {
                width: 64,
                height: 64,
                pixels: fresh,
            },
            &F32Image {
                width: 64,
                height: 64,
                pixels: frame.clone(),
            },
        );
    }
}
