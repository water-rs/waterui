//! Independent polygon coverage and full-scene oracle gates for GPU paths.
//!
//! Full curved/HDR scenes retain section 4.3 thresholds 1–3. Polygon coverage
//! is checked separately on identical f32 line geometry against the independent
//! f64 region-area oracle, as visually equivalent coverage images.
//! CPU/GPU painted-image differences remain diagnostics, because different
//! valid curve flattenings and HDR paint do not define an R8 coverage metric.

use std::borrow::Cow;
use std::path::{Path as FsPath, PathBuf};

use cherenkov::kurbo::{Affine, BezPath, PathEl, Stroke};
use cherenkov::{
    Draw as _, Engine, FillRule, Fixed, FrameTime, Offscreen, OffscreenFormat, PathRef, Semantic,
    Shape, WorkingColor,
};
use cherenkov_oracle::image::F32Image;
use cherenkov_scene::{Draw as SceneDraw, Item, Layer, Paint, Scene, Shape as SceneShape};

use super::CoverageRoute;
use crate::{Gpu, GpuConfig};

/// Explicit routes and the production selection policy.
const ROUTES: [CoverageRoute; 3] = [
    CoverageRoute::Sweep,
    CoverageRoute::Gpu,
    CoverageRoute::Automatic,
];

/// The large-path scenes of #2546 — a road network of 1,920 stroke-outline
/// crossings, a dense self-overlapping ring field under both fill rules,
/// and slivers whose device bounds are the whole surface — and the stroke
/// scenes of #2544: every join and cap kind, dashes, degenerate curves,
/// hairlines and a skewed root transform, each one stroked path above the
/// stroke candidate floor. All in P3-only and above-SDR-white colours.
const SCENES: &[&str] = &[
    "stroke-road-network-p3",
    "stroke-road-network-hdr",
    "path-dense-overlap-nonzero-p3",
    "path-dense-overlap-evenodd-p3",
    "path-dense-overlap-nonzero-hdr",
    "path-dense-overlap-evenodd-hdr",
    "path-sparse-bounds-nonzero-p3",
    "path-sparse-bounds-evenodd-hdr",
    "stroke-gpu-joins-miter-p3",
    "stroke-gpu-joins-miter-hdr",
    "stroke-gpu-joins-bevel-p3",
    "stroke-gpu-joins-bevel-hdr",
    "stroke-gpu-joins-round-p3",
    "stroke-gpu-joins-round-hdr",
    "stroke-gpu-joins-miter-lim15-p3",
    "stroke-gpu-joins-miter-lim15-hdr",
    "stroke-gpu-caps-square-round-p3",
    "stroke-gpu-caps-square-round-hdr",
    "stroke-gpu-caps-round-butt-p3",
    "stroke-gpu-caps-round-butt-hdr",
    "stroke-gpu-caps-butt-square-p3",
    "stroke-gpu-caps-butt-square-hdr",
    "stroke-gpu-dash-p3",
    "stroke-gpu-dash-hdr",
    "stroke-gpu-dash-zero-p3",
    "stroke-gpu-dash-zero-hdr",
    "stroke-gpu-dash-ties-p3",
    "stroke-gpu-dash-ties-hdr",
    "stroke-gpu-dash-dense-p3",
    "stroke-gpu-dash-dense-hdr",
    "stroke-gpu-curves-p3",
    "stroke-gpu-curves-hdr",
    "stroke-gpu-hairline-w025-p3",
    "stroke-gpu-hairline-w005-hdr",
    "stroke-gpu-transformed-p3",
    "stroke-gpu-transformed-hdr",
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

/// The scene's root-layer transform, which the engine's root layer takes.
/// A large-path scene's root carries a transform and its one draw and
/// nothing else; any other root-layer state is a scene this harness does
/// not describe.
fn root_transform(scene: &Scene) -> Affine {
    let rest = Layer {
        transform: Affine::IDENTITY,
        items: Vec::new(),
        ..scene.root.clone()
    };
    assert_eq!(
        rest,
        Layer::default(),
        "a large-path scene's root layer carries only a transform and its draw"
    );
    scene.root.transform
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
        tx[surface.root()].transform(root_transform(scene));
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

/// A coverage mask as a grey image, so coverage compares like any render.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the oracle's f64 coverage compares as f32 pixels"
)]
fn coverage_image(scene: &Scene, coverage: impl Iterator<Item = f64>) -> F32Image {
    F32Image {
        width: scene.width,
        height: scene.height,
        pixels: coverage.map(|area| [area as f32; 4]).collect(),
    }
}

fn check_polygon_coverage(scene: &Scene, name: &str) {
    let (polygon, reference) = polygon_coverage(scene);
    let image = render_path(
        scene,
        polygon,
        WorkingColor::new([1.0; 4]),
        None,
        WorkingColor::new([0.0; 4]),
        CoverageRoute::Gpu,
    );
    assert_eq!(
        image.pixels.len(),
        reference.len(),
        "complete coverage image"
    );
    for (index, pixel) in image.pixels.iter().enumerate() {
        assert!(
            pixel[3].is_finite() && (0.0..=1.0).contains(&pixel[3]),
            "{name}: invalid coverage at pixel {index}"
        );
    }
    cherenkov_oracle::metrics::assert_images_match(
        &coverage_image(scene, reference.into_iter()),
        &coverage_image(scene, image.pixels.iter().map(|pixel| f64::from(pixel[3]))),
    );
}

/// One scene's oracle-relative metrics on each of [`ROUTES`].
fn route_metrics(name: &str) -> [cherenkov_oracle::Metrics; 3] {
    let scene = Scene::load(&corpus().join(name)).expect("load corpus scene");
    let reference = cherenkov_oracle::metrics::Reference::new(
        &cherenkov_oracle::Renderer::new(
            usize::try_from(scene.width).expect("scene width"),
            usize::try_from(scene.height).expect("scene height"),
        )
        .render(&scene, &corpus().join(name))
        .expect("oracle render"),
    );
    ROUTES.map(|route| reference.metrics(&render(&scene, route)))
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
    // Every scene's per-route oracle metrics, written as TOML to the file
    // `CHERENKOV_ROUTE_METRICS` names: the record a route change is
    // compared against.
    let mut record = toml::Table::new();
    for name in SCENES {
        let measured = route_metrics(name);
        let mut routes = toml::Table::new();
        for (i, (route, metrics)) in ROUTES.iter().zip(measured).enumerate() {
            oracle_metrics[i].push(metrics.flip_mean);
            routes.insert(
                format!("{route:?}"),
                toml::Value::Table(toml::Table::from_iter([
                    ("flip_mean".to_owned(), metrics.flip_mean.into()),
                    ("flip_max".to_owned(), metrics.flip_max.into()),
                    ("max_local_error".to_owned(), metrics.max_local_error.into()),
                ])),
            );
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
        record.insert((*name).to_owned(), toml::Value::Table(routes));
    }
    if let Some(path) = std::env::var_os("CHERENKOV_ROUTE_METRICS") {
        std::fs::write(
            path,
            toml::to_string(&record).expect("serialize the route metrics"),
        )
        .expect("write the route metrics");
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
        let mut expected_grid = Vec::with_capacity(64);
        for y in 0..8usize {
            for x in 0..8usize {
                let expected: f32 = if x == 1 && y == 1 {
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
                expected_grid.push(expected);
            }
        }
        let alpha: Vec<f32> = image.pixels.iter().map(|pixel| pixel[3]).collect();
        cherenkov_oracle::metrics::assert_close(&alpha, &expected_grid);
    }
}

/// Stroke-outline polygons the GPU route used to misrender
/// (water-rs/waterui#3158): a butt cap whose edge passes within rounding of
/// a pixel corner, and two outlines that retrace an offset line, so distinct
/// segments clip to one line in a cell. Each is its own path, so its grid —
/// and with it the f32 rounding the cells see — is the one the outline had.
#[test]
fn gpu_coverage_matches_rounding_degenerate_outlines() {
    const SIZE: (u32, u32) = (320, 160);
    const POLYGONS: [&[(f32, f32)]; 3] = [
        &[
            (3.515_190_4, 139.125_38),
            (9.515_19, 135.799_53),
            (10.484_81, 137.548_77),
            (4.484_809_4, 140.874_62),
        ],
        &[
            (10.815_504, 3.474_928_6),
            (17.248_934, 4.561_709),
            (14.892_125, 6.918_518),
            (14.951_047, 5.162_539_5),
            (15.799_575, 6.011_067_4),
            (14.892_125, 6.918_518),
        ],
        &[
            (306.657_75, 141.152_2),
            (305.458_37, 141.112_95),
            (305.581_54, 137.350_5),
            (292.057, 149.402_47),
            (305.458_37, 141.112_95),
            (305.505_8, 139.664_28),
        ],
    ];
    let engine = Engine::<Gpu>::new(GpuConfig {
        coverage_route: Some(CoverageRoute::Gpu),
        ..GpuConfig::default()
    })
    .expect("GPU coverage test adapter");
    let surface = engine
        .surface(Offscreen::new(SIZE, OffscreenFormat::LinearF16), || {})
        .expect("coverage test surface");
    surface.clear_color(WorkingColor::new([0.0; 4]));
    let mut oracle = cherenkov_oracle::coverage::Coverage::new(SIZE.0 as usize, SIZE.1 as usize);
    let paths = POLYGONS.map(|polygon| {
        let points: Vec<(f64, f64)> = polygon
            .iter()
            .map(|&(x, y)| (f64::from(x), f64::from(y)))
            .collect();
        oracle.add_polyline(&points, true);
        let mut path = BezPath::new();
        path.move_to(points[0]);
        for &point in &points[1..] {
            path.line_to(point);
        }
        path.close_path();
        path
    });
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|canvas| {
            for path in paths {
                canvas.fill(
                    Fixed(RuledPath {
                        elements: path.into_iter().collect(),
                        rule: FillRule::NonZero,
                    }),
                    WorkingColor::new([1.0; 4]),
                );
            }
        }));
    });
    engine
        .render(FrameTime::now())
        .expect("GPU outline coverage");
    let image = surface.readback().expect("coverage pixels");
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the oracle's f64 coverage compares as f32 pixels"
    )]
    let reference = F32Image {
        width: SIZE.0,
        height: SIZE.1,
        pixels: oracle
            .finish(cherenkov_scene::FillRule::NonZero)
            .into_iter()
            .map(|area| [area as f32; 4])
            .collect(),
    };
    cherenkov_oracle::metrics::assert_images_match(
        &reference,
        &F32Image {
            width: SIZE.0,
            height: SIZE.1,
            pixels: image.pixels,
        },
    );
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
            // The edge's coverage is an 8-bit mask value, 191/255 for 0.75.
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "coverage is in [0, 1]"
            )]
            let edge_byte = (edge[3] * 255.0).round() as u8;
            cherenkov_oracle::metrics::assert_unorm8_close(&[edge_byte], &[191]);
            for x in [32usize, 128, 240] {
                cherenkov_oracle::metrics::assert_close(&image.pixels[4 * 273 + x], &expected);
            }
            cherenkov_oracle::metrics::assert_close(&image.pixels[0], &[0.0; 4]);
            cherenkov_oracle::metrics::assert_close(&image.pixels[8 * 257 + 256], &[0.0; 4]);
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

/// A render's pixels as an image of `size`.
fn image(size: (u32, u32), pixels: Vec<[f32; 4]>) -> F32Image {
    F32Image {
        width: size.0,
        height: size.1,
        pixels,
    }
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
    // The star covers its centre.
    cherenkov_oracle::metrics::assert_close(&[sweep[(2 + 32) * 64 + 3 + 32][0]], &[1.0]);
    cherenkov_oracle::metrics::assert_images_match(&image((64, 64), sweep), &image((64, 64), gpu));
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
    // The circle covers its centre.
    cherenkov_oracle::metrics::assert_close(&[sweep[192 * 320 + 192][0]], &[1.0]);
    cherenkov_oracle::metrics::assert_images_match(
        &image((320, 320), sweep),
        &image((320, 320), gpu),
    );
}
