//! Route equivalence on the large-path corpus scenes (water-rs/waterui#2546).
//!
//! The coverage specification's section 4.3 threshold 4 is stronger than the
//! oracle gate: for any scene a route accepts, the same scene forced through
//! another route must agree with it to `flip_mean <= 0.0005` and
//! `max_local_error <= 0.004` *between the two engine images* — quantization
//! noise, not a visible difference. This module is that comparison, and it
//! lives inside the crate because decision T2 keeps route forcing out of
//! public API: [`force_route`] is `#[cfg(test)]`, so no consumer can reach
//! it, and an integration test could not either.
//!
//! [`CoverageRoute::Sweep`] is the only route today, so the pair compared
//! here is the sweep against itself: the harness, the scenes and the
//! thresholds are what #2546 lands, and water-rs/waterui#2543 adds its
//! variant to `ROUTES` with the route itself. Comparing one route with
//! itself is not vacuous — it renders the eight large-path scenes twice
//! through the whole engine and holds the pair to the threshold, which fails
//! on any frame-to-frame non-determinism in lowering, batching or the atlas.
//!
//! Thresholds 1–3 of the same section compare a route against the oracle,
//! not against another route; they need the oracle's `f64` reference, which
//! the bench harness renders, and they are gated in
//! `bench/tests/large_paths.rs`.

use std::borrow::Cow;
use std::path::{Path as FsPath, PathBuf};

use cherenkov::kurbo::{BezPath, PathEl, Stroke};
use cherenkov::{
    Draw as _, Engine, EngineError, FillRule, Fixed, FrameTime, Offscreen, OffscreenFormat,
    PathRef, Semantic, Shape, WorkingColor,
};
use cherenkov_oracle::image::F32Image;
use cherenkov_scene::{Draw as SceneDraw, Item, Paint, Scene, Shape as SceneShape};

use super::{CoverageRoute, force_route};
use crate::{Gpu, GpuConfig};

/// Section 4.3 threshold 4: the two routes' images.
const FLIP_MEAN: f64 = 0.0005;
/// Section 4.3 threshold 4: the two routes' images.
const MAX_LOCAL_ERROR: f64 = 0.004;

/// Every route the differential pairs. #2543 adds its variant here.
const ROUTES: &[CoverageRoute] = &[CoverageRoute::Sweep];

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
    FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../scenes/corpus")
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
fn geometry(draw: &SceneDraw) -> (BezPath, FillRule, WorkingColor, Option<f64>) {
    let (shape, paint, rule, width) = match draw {
        SceneDraw::Fill { shape, rule, paint } => (shape, paint, *rule, None),
        SceneDraw::Stroke {
            shape,
            stroke,
            paint,
        } => {
            assert_eq!(
                (
                    stroke.join,
                    stroke.start_cap,
                    stroke.end_cap,
                    stroke.dash_pattern.as_slice()
                ),
                (
                    cherenkov_scene::StrokeStyle::default().join,
                    cherenkov_scene::StrokeStyle::default().start_cap,
                    cherenkov_scene::StrokeStyle::default().end_cap,
                    [].as_slice()
                ),
                "the large-path strokes use the default joins and caps",
            );
            (
                shape,
                paint,
                cherenkov_scene::FillRule::NonZero,
                Some(stroke.width),
            )
        }
        other => panic!("a large-path scene fills or strokes, not {other:?}"),
    };
    let SceneShape::Path { path } = shape else {
        panic!("a large-path scene draws a path, not {shape:?}");
    };
    let Paint::Solid(color) = paint else {
        panic!("a large-path scene paints solid, not {paint:?}");
    };
    (path.clone(), core_rule(rule), working(color), width)
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

/// Renders `scene`'s single draw through `route`, or `None` when no adapter
/// exists on this machine.
fn render(scene: &Scene, route: CoverageRoute) -> Option<F32Image> {
    let engine = match Engine::<Gpu>::new(GpuConfig::default()) {
        Ok(engine) => engine,
        Err(EngineError::Backend(_)) => return None,
        Err(e) => panic!("engine init failed: {e}"),
    };
    let surface = engine
        .surface(
            Offscreen::new((scene.width, scene.height), OffscreenFormat::LinearF16),
            || {},
        )
        .expect("offscreen surface");
    surface.clear_color(working(&scene.clear));
    let (path, rule, ink, width) = geometry(single_draw(scene));
    let shape = RuledPath {
        elements: path.into_iter().collect(),
        rule,
    };
    // The override is thread-local and the engine lowers on this thread:
    // `render` below is where `rasterize` reads it.
    force_route(Some(route));
    surface.update(|tx| {
        tx[surface.root()].content(surface.record(|c| match width {
            None => c.fill(Fixed(shape), ink),
            Some(width) => c.stroke(
                Fixed(shape),
                Stroke {
                    width,
                    ..Stroke::default()
                },
                ink,
            ),
        }));
    });
    let frame = engine.render(FrameTime::now());
    force_route(None);
    frame.expect("render the forced route");
    let readback = surface.readback().expect("readback");
    Some(F32Image {
        width: scene.width,
        height: scene.height,
        pixels: readback.pixels,
    })
}

/// The route pairs one scene is rendered through: every consecutive pair of
/// [`ROUTES`], or the single route against itself while it is the only one.
fn pairs() -> Vec<(CoverageRoute, CoverageRoute)> {
    match ROUTES {
        [only] => vec![(*only, *only)],
        many => many.windows(2).map(|w| (w[0], w[1])).collect(),
    }
}

#[test]
fn routes_agree_on_the_large_path_scenes() {
    let pairs = pairs();
    let mut failures = Vec::new();
    let mut compared = 0usize;
    for name in SCENES {
        let scene = Scene::load(&corpus().join(name)).expect("load corpus scene");
        for (a, b) in &pairs {
            let (Some(left), Some(right)) = (render(&scene, *a), render(&scene, *b)) else {
                eprintln!("no GPU adapter: route equivalence not measured");
                return;
            };
            let (metrics, _) = cherenkov_oracle::metrics::compare(&left, &right);
            compared += 1;
            if metrics.flip_mean > FLIP_MEAN || metrics.max_local_error > MAX_LOCAL_ERROR {
                failures.push(format!(
                    "{name} {a:?} vs {b:?}: flip_mean {:.6}, max_local_error {:.6}",
                    metrics.flip_mean, metrics.max_local_error
                ));
            }
        }
    }
    assert_eq!(
        compared,
        SCENES.len() * pairs.len(),
        "every scene compared on every route pair"
    );
    assert!(
        failures.is_empty(),
        "route equivalence over threshold 4 (flip_mean {FLIP_MEAN}, max_local_error {MAX_LOCAL_ERROR}): {}",
        failures.join("; ")
    );
}

#[test]
fn the_override_decides_the_route() {
    for &forced in ROUTES {
        force_route(Some(forced));
        assert_eq!(super::route(), forced);
    }
    force_route(None);
    assert_eq!(super::route(), CoverageRoute::Sweep);
}
