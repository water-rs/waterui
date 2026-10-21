//! Independent polygon coverage and full-scene oracle gates for GPU paths.
//!
//! Full curved/HDR scenes retain section 4.3 thresholds 1–3. Polygon coverage
//! is checked separately on identical f32 line geometry against the independent
//! f64 region-area oracle: at most one R8 byte of error and exact interiors.
//! CPU/GPU painted-image differences remain diagnostics, because different
//! valid curve flattenings and HDR paint do not define an R8 coverage metric.

use std::borrow::Cow;
use std::path::{Path as FsPath, PathBuf};

use cherenkov::kurbo::{BezPath, PathEl, Stroke};
use cherenkov::{
    Draw as _, Engine, FillRule, Fixed, FrameTime, Offscreen, OffscreenFormat, PathRef, Semantic,
    Shape, WorkingColor,
};
use cherenkov_oracle::image::F32Image;
use cherenkov_scene::{Draw as SceneDraw, Item, Paint, Scene, Shape as SceneShape};

use super::CoverageRoute;
use crate::{Gpu, GpuConfig};

/// Explicit routes and the production selection policy.
const ROUTES: [CoverageRoute; 3] = [
    CoverageRoute::Sweep,
    CoverageRoute::Gpu,
    CoverageRoute::Automatic,
];

/// The large-path scenes of #2546: a road network of 1,920 stroke-outline
/// crossings, a dense self-overlapping ring field under both fill rules,
/// and slivers whose device bounds are the whole surface, in P3-only and
/// above-SDR-white colours.
const SCENES: &[&str] = &[
    "stroke-road-network-p3",
    "stroke-road-network-hdr",
    "path-dense-overlap-nonzero-p3",
    "path-dense-overlap-evenodd-p3",
    "path-dense-overlap-nonzero-hdr",
    "path-dense-overlap-evenodd-hdr",
    "path-sparse-bounds-nonzero-p3",
    "path-sparse-bounds-evenodd-hdr",
];

fn corpus() -> PathBuf {
    std::env::var_os("CHERENKOV_TEST_CORPUS").map_or_else(
        || FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../scenes/corpus"),
        PathBuf::from,
    )
}

/// A path carrying its fill rule, which `kurbo::Shape` cannot: the blanket
/// `Shape` impl fills every path non-zero.
struct RuledPath {
    elements: Vec<PathEl>,
    rule: FillRule,
}

impl Shape for RuledPath {
    fn semantic(&self) -> Semantic<'_> {
        Semantic::Path(PathRef {
            elements: Cow::Borrowed(&self.elements),
            rule: self.rule,
        })
    }
}

/// The one fill or stroke of a large-path scene. These scenes are one draw
/// on a cleared surface by construction; anything else is a scene this
/// harness does not describe, and is an error rather than a partial render.
fn single_draw(scene: &Scene) -> &SceneDraw {
    let [Item::Draw(draw)] = &scene.root.items[..] else {
        panic!("a large-path scene is one draw, not {:?}", scene.root.items);
    };
    draw
}

/// The draw's path and its paint colour as the engine takes them.
fn geometry(draw: &SceneDraw) -> (BezPath, FillRule, WorkingColor, Option<Stroke>) {
    let (shape, paint, rule, stroke) = match draw {
        SceneDraw::Fill { shape, rule, paint } => (shape, paint, *rule, None),
        SceneDraw::Stroke {
            shape,
            stroke,
            paint,
        } => (
            shape,
            paint,
            cherenkov_scene::FillRule::NonZero,
            Some(Stroke::from(stroke)),
        ),
        other => panic!("a large-path scene fills or strokes, not {other:?}"),
    };
    let SceneShape::Path { path } = shape else {
        panic!("a large-path scene draws a path, not {shape:?}");
    };
    let Paint::Solid(color) = paint else {
        panic!("a large-path scene paints solid, not {paint:?}");
    };
    (path.clone(), core_rule(rule), working(color), stroke)
}

const fn core_rule(rule: cherenkov_scene::FillRule) -> FillRule {
    match rule {
        cherenkov_scene::FillRule::NonZero => FillRule::NonZero,
        cherenkov_scene::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}

/// A scene colour as the engine's straight-alpha working colour, through the
/// oracle's own conversion so the harness and the reference agree.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the working space is f32 at the engine boundary"
)]
fn working(color: &cherenkov_scene::Color) -> WorkingColor {
    let [r, g, b, a] = cherenkov_oracle::color::to_working(color);
    let (r, g, b) = if a > 1e-12 {
        (r / a, g / a, b / a)
    } else {
        (0.0, 0.0, 0.0)
    };
    WorkingColor::new([r as f32, g as f32, b as f32, a as f32])
}

/// Render one path; a missing adapter is not a successful device gate.
fn render_path(
    scene: &Scene,
    shape: RuledPath,
    ink: WorkingColor,
    stroke: Option<&Stroke>,
    clear: WorkingColor,
    route: CoverageRoute,
) -> F32Image {
    let engine = Engine::<Gpu>::new(GpuConfig {
        coverage_route: Some(route),
        ..GpuConfig::default()
    })
    .expect("GPU coverage test adapter");
    let surface = engine
        .surface(
            Offscreen::new((scene.width, scene.height), OffscreenFormat::LinearF16),
            || {},
        )
        .expect("offscreen surface");
    surface.clear_color(clear);
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| match stroke {
            None => c.fill(Fixed(shape), ink),
            Some(stroke) => c.stroke(Fixed(shape), stroke.clone(), ink),
        }));
    });
    let frame = engine.render(FrameTime::now());
    frame.expect("render the forced route");
    let readback = surface.readback().expect("readback");
    F32Image {
        width: scene.width,
        height: scene.height,
        pixels: readback.pixels,
    }
}

fn render(scene: &Scene, route: CoverageRoute) -> F32Image {
    let (path, rule, ink, stroke) = geometry(single_draw(scene));
    render_path(
        scene,
        RuledPath {
            elements: path.into_iter().collect(),
            rule,
        },
        ink,
        stroke.as_ref(),
        working(&scene.clear),
        route,
    )
}

/// One canonical polygon is supplied to both coverage routes and the
/// independent oracle. Its f32 endpoints are exactly representable in f64.
#[expect(
    clippy::cast_possible_truncation,
    reason = "canonical device geometry is explicitly f32"
)]
fn polygon_coverage(scene: &Scene) -> (RuledPath, Vec<f64>) {
    let (path, rule, _, stroke) = geometry(single_draw(scene));
    let path = stroke.map_or_else(
        || path.clone(),
        |stroke| {
            cherenkov::kurbo::stroke(
                path.clone(),
                &stroke,
                &cherenkov::kurbo::StrokeOpts::default(),
                super::FLATTEN,
            )
        },
    );
    let mut oracle =
        cherenkov_oracle::coverage::Coverage::new(scene.width as usize, scene.height as usize);
    let mut polygon = BezPath::new();
    for contour in cherenkov_oracle::path::flatten_at(&path, super::FLATTEN) {
        let points: Vec<_> = contour
            .points
            .into_iter()
            .map(|(x, y)| (f64::from(x as f32), f64::from(y as f32)))
            .collect();
        if let Some((&first, rest)) = points.split_first() {
            polygon.move_to(first);
            for &point in rest {
                polygon.line_to(point);
            }
            polygon.close_path();
            oracle.add_polyline(&points, true);
        }
    }
    let oracle_rule = match rule {
        FillRule::NonZero => cherenkov_scene::FillRule::NonZero,
        FillRule::EvenOdd => cherenkov_scene::FillRule::EvenOdd,
    };
    (
        RuledPath {
            elements: polygon.into_iter().collect(),
            rule,
        },
        oracle.finish(oracle_rule),
    )
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped R8 coverage"
)]
fn quantize(area: f64) -> u8 {
    area.clamp(0.0, 1.0).mul_add(255.0, 0.5) as u8
}

#[expect(
    clippy::float_cmp,
    reason = "exact full/empty oracle interiors have a stronger byte contract"
)]
fn check_polygon_coverage(scene: &Scene, name: &str) {
    let (polygon, reference) = polygon_coverage(scene);
    let images = ROUTES.map(|route| {
        render_path(
            scene,
            RuledPath {
                elements: polygon.elements.clone(),
                rule: polygon.rule,
            },
            WorkingColor::new([1.0; 4]),
            None,
            WorkingColor::new([0.0; 4]),
            route,
        )
    });
    for (route, image) in ROUTES.iter().zip(images) {
        assert_eq!(
            image.pixels.len(),
            reference.len(),
            "complete coverage image"
        );
        let mut mismatches = 0;
        let mut worst = (0u8, 0usize, 0u8, 0u8);
        for (index, (&area, pixel)) in reference.iter().zip(&image.pixels).enumerate() {
            assert!(
                pixel[3].is_finite() && (0.0..=1.0).contains(&pixel[3]),
                "{name}: invalid coverage at pixel {index}"
            );
            let expected = quantize(area);
            // LinearF16 alpha uniquely preserves every R8 value: its
            // rounding error is less than half of one coverage byte.
            let actual = quantize(f64::from(pixel[3]));
            let difference = expected.abs_diff(actual);
            if difference > worst.0 {
                worst = (difference, index, expected, actual);
            }
            if difference > 1 || ((area == 0.0 || area == 1.0) && actual != expected) {
                mismatches += 1;
            }
        }
        if *route == CoverageRoute::Gpu {
            assert_eq!(
                mismatches, 0,
                "{name}: GPU polygon coverage; worst(byte error,index,expected,actual)={worst:?}"
            );
        } else if mismatches > 0 {
            tracing::warn!(
                scene = name,
                mismatches,
                ?worst,
                "CPU raster does not conform to exact polygon coverage"
            );
        }
    }
}

#[test]
fn gpu_paths_satisfy_polygon_and_scene_oracles() {
    // Section 4.3 thresholds 1–3 against the oracle: the absolute ceilings
    // of threshold 2, threshold 3's measured corpus maxima (which the
    // ceilings already tighten here), and threshold 1's regression band —
    // the CPU sweep supplies the baseline for both explicit GPU coverage
    // and automatic selection.
    const FLIP_MEAN_CEIL: f64 = 0.006;
    const FLIP_MAX_CEIL: f64 = 0.15;
    const MAX_LOCAL_CEIL: f64 = 0.09;

    let mut failures = Vec::new();
    // [route][scene] oracle-relative metrics, for threshold 1.
    let mut oracle_metrics: [Vec<f64>; 3] = std::array::from_fn(|_| Vec::new());
    for name in SCENES {
        let scene = Scene::load(&corpus().join(name)).expect("load corpus scene");
        let reference = cherenkov_oracle::Renderer::new(
            usize::try_from(scene.width).expect("scene width"),
            usize::try_from(scene.height).expect("scene height"),
        )
        .render(&scene, &corpus().join(name))
        .expect("oracle render");
        let images = ROUTES.map(|route| render(&scene, route));
        for (i, (route, image)) in ROUTES.iter().zip(images).enumerate() {
            let (metrics, _) = cherenkov_oracle::metrics::compare(&reference, &image);
            oracle_metrics[i].push(metrics.flip_mean);
            if metrics.flip_mean > FLIP_MEAN_CEIL
                || metrics.flip_max > FLIP_MAX_CEIL
                || metrics.max_local_error > MAX_LOCAL_CEIL
            {
                failures.push(format!(
                    "{name} {route:?} vs oracle: flip_mean {:.6}, flip_max {:.6}, max_local_error {:.6}",
                    metrics.flip_mean, metrics.flip_max, metrics.max_local_error
                ));
            }
        }
    }
    for name in [
        "stroke-road-network-p3",
        "path-dense-overlap-nonzero-p3",
        "path-dense-overlap-evenodd-p3",
        "path-sparse-bounds-nonzero-p3",
        "path-sparse-bounds-evenodd-hdr",
    ] {
        let scene = Scene::load(&corpus().join(name)).expect("load polygon coverage scene");
        check_polygon_coverage(&scene, name);
    }
    // Threshold 1: per scene, the GPU route's flip_mean stays inside the
    // 5%-or-0.0005 band of the sweep's. Aggregate means are diagnostic:
    // improving the sweep must not reject an unchanged GPU route whose
    // per-scene and independent polygon-oracle checks still pass.
    let [sweep, gpu, automatic] = &oracle_metrics;
    for (route, selected) in [
        (CoverageRoute::Gpu, gpu),
        (CoverageRoute::Automatic, automatic),
    ] {
        for (name, (&cpu, &selected_mean)) in SCENES.iter().zip(sweep.iter().zip(selected.iter())) {
            if selected_mean > f64::max(cpu * 1.05, cpu + 0.0005) {
                failures.push(format!(
                    "{name} {route:?} regression band: sweep {cpu:.6} -> {selected_mean:.6}"
                ));
            }
        }
    }
    let sweep_mean =
        sweep.iter().sum::<f64>() / f64::from(u32::try_from(sweep.len()).expect("scene count"));
    let gpu_mean =
        gpu.iter().sum::<f64>() / f64::from(u32::try_from(gpu.len()).expect("scene count"));
    tracing::info!(
        sweep_mean,
        gpu_mean,
        "aggregate oracle-relative route error (diagnostic)"
    );
    assert!(
        failures.is_empty(),
        "polygon coverage and oracle gates over section 4.3: {}",
        failures.join("; ")
    );
}

/// Analytic cases that distinguish region area from folded integrated
/// winding, and exercise integer-corner ownership across a sparse row.
#[test]
fn gpu_boundary_cells_integrate_fill_rule_regions() {
    let engine = Engine::<Gpu>::new(GpuConfig {
        coverage_route: Some(CoverageRoute::Gpu),
        ..GpuConfig::default()
    })
    .expect("GPU coverage test adapter");
    for rule in [FillRule::NonZero, FillRule::EvenOdd] {
        let surface = engine
            .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {})
            .expect("coverage test surface");
        surface.clear_color(WorkingColor::new([0.0; 4]));
        let mut duplicate = BezPath::new();
        for _ in 0..2 {
            duplicate.move_to((1.0, 1.0));
            duplicate.line_to((1.5, 1.0));
            duplicate.line_to((1.5, 2.0));
            duplicate.line_to((1.0, 2.0));
            duplicate.close_path();
        }
        let mut triangle = BezPath::new();
        triangle.move_to((3.0, 3.0));
        triangle.line_to((7.0, 3.0));
        triangle.line_to((3.0, 7.0));
        triangle.close_path();
        surface.update(|tx| {
            tx[surface.root()].content(surface.record(|canvas| {
                for path in [duplicate, triangle] {
                    canvas.fill(
                        Fixed(RuledPath {
                            elements: path.into_iter().collect(),
                            rule,
                        }),
                        WorkingColor::new([1.0; 4]),
                    );
                }
            }));
        });
        engine
            .render(FrameTime::now())
            .expect("GPU boundary integration");
        let image = surface.readback().expect("coverage pixels");
        for y in 0..8usize {
            for x in 0..8usize {
                let expected = if x == 1 && y == 1 {
                    if rule == FillRule::NonZero {
                        128.0 / 255.0
                    } else {
                        0.0
                    }
                } else if x >= 3 && y >= 3 && x + y < 9 {
                    1.0
                } else if x >= 3 && y >= 3 && x + y == 9 {
                    128.0 / 255.0
                } else {
                    0.0
                };
                let alpha = image.pixels[y * 8 + x][3];
                assert!(
                    (alpha - expected).abs() < 0.001,
                    "{rule:?} ({x},{y}): coverage {alpha}, expected {expected}; alpha grid {:?}",
                    image
                        .pixels
                        .iter()
                        .map(|pixel| pixel[3])
                        .collect::<Vec<_>>()
                );
            }
        }
    }
}

/// Sparse full interiors, translucent painter ordering and repeated frames
/// share the same GPU batch without reusing stale indirect emissions.
#[test]
fn gpu_sparse_interiors_preserve_painter_order_across_frames() {
    let engine = Engine::<Gpu>::new(GpuConfig {
        coverage_route: Some(CoverageRoute::Gpu),
        ..GpuConfig::default()
    })
    .expect("GPU coverage test adapter");
    let surface = engine
        .surface(Offscreen::new((273, 9), OffscreenFormat::LinearF16), || {})
        .expect("coverage test surface");
    surface.clear_color(WorkingColor::new([0.0; 4]));
    let mut outer = BezPath::new();
    outer.move_to((1.25, 1.25));
    outer.line_to((255.75, 1.25));
    outer.line_to((255.75, 7.75));
    outer.line_to((1.25, 7.75));
    outer.close_path();
    let mut inner = BezPath::new();
    inner.move_to((4.5, 2.5));
    inner.line_to((250.5, 2.5));
    inner.line_to((250.5, 6.5));
    inner.line_to((4.5, 6.5));
    inner.close_path();
    for (reverse, red, shift) in [
        (false, true, 0u32),
        (true, true, 0),
        (false, false, 1),
        (false, false, 20),
        (false, true, 0),
    ] {
        surface.update(|tx| {
            tx[surface.root()]
                .transform(cherenkov::kurbo::Affine::translate((f64::from(shift), 0.0)));
            tx[surface.root()].content(surface.record(|canvas| {
                let mut layers = [
                    (
                        outer.clone(),
                        WorkingColor::new(if red {
                            [1.0, 0.0, 0.0, 1.0]
                        } else {
                            [0.0, 1.0, 0.0, 1.0]
                        }),
                    ),
                    (inner.clone(), WorkingColor::new([0.0, 0.0, 1.0, 0.5])),
                ];
                if reverse {
                    layers.reverse();
                }
                for (path, color) in layers {
                    canvas.fill(
                        Fixed(RuledPath {
                            elements: path.into_iter().collect(),
                            rule: FillRule::NonZero,
                        }),
                        color,
                    );
                }
            }));
        });
        for _ in 0..2 {
            engine.render(FrameTime::now()).expect("GPU ordered frame");
            let image = surface.readback().expect("ordered coverage pixels");
            let expected = if reverse {
                [1.0, 0.0, 0.0, 1.0]
            } else if red {
                [0.5, 0.0, 0.5, 1.0]
            } else {
                [0.0, 0.5, 0.5, 1.0]
            };
            let edge = image.pixels[4 * 273 + 1 + usize::try_from(shift).unwrap()];
            assert!(
                (edge[3] - 0.75).abs() < 0.005,
                "translated outer edge: {edge:?}"
            );
            for x in [32usize, 128, 240] {
                let actual = image.pixels[4 * 273 + x];
                for channel in 0..4 {
                    assert!(
                        (actual[channel] - expected[channel]).abs() < 0.001,
                        "reverse={reverse} x={x}: {actual:?}, expected {expected:?}"
                    );
                }
            }
            assert_eq!(image.pixels[0], [0.0; 4]);
            assert_eq!(image.pixels[8 * 257 + 256], [0.0; 4]);
        }
    }
}

/// Render a recorded scene on `route`, returning its pixels.
fn render_recorded(
    route: CoverageRoute,
    size: (u32, u32),
    record: impl FnOnce(&mut cherenkov::Recorder),
) -> Vec<[f32; 4]> {
    let engine = Engine::<Gpu>::new(GpuConfig {
        coverage_route: Some(route),
        ..GpuConfig::default()
    })
    .expect("GPU coverage test adapter");
    let surface = engine
        .surface(Offscreen::new(size, OffscreenFormat::LinearF16), || {})
        .expect("offscreen surface");
    surface.clear_color(WorkingColor::new([0.0, 0.0, 0.0, 1.0]));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(record));
    });
    engine.render(FrameTime::now()).expect("render the route");
    surface.readback().expect("readback").pixels
}

/// A five-point star centred on (32, 32) with straight edges, whose
/// coverage both routes compute exactly.
fn star() -> BezPath {
    let mut path = BezPath::new();
    for i in 0..5 {
        let angle = f64::from(i).mul_add(144.0, -90.0).to_radians();
        let point = (
            angle.cos().mul_add(24.0, 32.0),
            angle.sin().mul_add(24.0, 32.0),
        );
        if i == 0 {
            path.move_to(point);
        } else {
            path.line_to(point);
        }
    }
    path.close_path();
    path
}

/// The worst per-channel difference between two renders.
fn worst_difference(a: &[[f32; 4]], b: &[[f32; 4]]) -> f32 {
    assert_eq!(a.len(), b.len(), "same surface");
    a.iter()
        .zip(b)
        .flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs()))
        .fold(0.0, f32::max)
}

/// A GPU-routed path drawn at a non-zero integer translation lands where
/// the polygon route puts it: the retained entry holds placement-relative
/// coverage, so the draw must add its own offset back on the frame that
/// generates it (water-rs/waterui#3081).
#[test]
fn a_gpu_path_at_an_integer_translation_matches_the_sweep_route() {
    let ink = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
    let draw = |c: &mut cherenkov::Recorder| {
        c.transform(cherenkov::kurbo::Affine::translate((3.0, 2.0)), |c| {
            c.fill(star(), ink);
        });
    };
    let sweep = render_recorded(CoverageRoute::Sweep, (64, 64), draw);
    let gpu = render_recorded(CoverageRoute::Gpu, (64, 64), draw);
    assert_eq!(
        sweep[(2 + 32) * 64 + 3 + 32][0],
        1.0,
        "the star covers its centre"
    );
    let worst = worst_difference(&sweep, &gpu);
    assert!(
        worst <= 2.0 / 255.0,
        "GPU placement differs from the sweep: {worst}"
    );
}

/// The polygon tiers of an opaque ellipse draw under the plain opaque
/// pipeline even when the pass's last range was GPU-routed: under the
/// cell-buffer pipeline every tier vertex past the eighth reads as a GPU
/// join and writes garbage depth (water-rs/waterui#3081).
#[test]
fn polygon_tiers_after_a_gpu_path_range_match_the_sweep_route() {
    let red = WorkingColor::new([1.0, 0.0, 0.0, 1.0]);
    let green = WorkingColor::new([0.0, 1.0, 0.0, 1.0]);
    let blue = WorkingColor::new([0.0, 0.0, 1.0, 1.0]);
    // The leading instances carry colours of their own: a tier vertex
    // misread as a GPU join takes its drawing state from one of them.
    let draw = |c: &mut cherenkov::Recorder| {
        c.fill(
            cherenkov::kurbo::Rect::new(300.0, 300.0, 316.0, 316.0),
            blue,
        );
        c.fill(cherenkov::kurbo::Circle::new((192.0, 192.0), 128.0), red);
        let mut square = BezPath::new();
        square.move_to((4.5, 4.5));
        square.line_to((60.5, 4.5));
        square.line_to((60.5, 60.5));
        square.line_to((4.5, 60.5));
        square.close_path();
        c.fill(square, green);
    };
    let sweep = render_recorded(CoverageRoute::Sweep, (320, 320), draw);
    let gpu = render_recorded(CoverageRoute::Gpu, (320, 320), draw);
    assert_eq!(
        sweep[192 * 320 + 192][0],
        1.0,
        "the circle covers its centre"
    );
    let worst = worst_difference(&sweep, &gpu);
    assert!(
        worst <= 2.0 / 255.0,
        "GPU tiers differ from the sweep: {worst}"
    );
}
