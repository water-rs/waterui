//! The box signed distance in `shared.wgsl` is the exact Euclidean distance
//! for elliptical and continuous (Lamé) corners at every depth.
//!
//! `sdf_sample` runs in the shader evaluator, in `f32`, at pixel centres
//! within 40 px of the boundary, inside and out, and is compared against
//! the oracle's `f64` distance to the true boundary. The deep interior of
//! a continuous-corner rect, where a projection along the implicit
//! gradient diverges, is part of the band.
//!
//! The shapes have equal corner radii and `sdf_sample` folds the sign of
//! the point before anything else, so one closed quadrant covers every
//! corner; the quadrant's axis lines, which the corner solve owns, are
//! sampled. Every other pixel centre is taken because the evaluator costs
//! milliseconds per sample in a debug build.

use cherenkov::kurbo::{Ellipse, Rect};
use cherenkov_oracle::sdf::{BoxShape, box_params, distance_and_normal};
use cherenkov_scene::{ContinuousRect, Shape};
use cherenkov_shader::eval::{Eval, Value};

/// The band of signed distances checked, in pixels.
const BAND: f64 = 40.0;
/// The largest distance error accepted, in pixels: `f32` rounding of
/// coordinates around 100 px.
const MAX_DISTANCE_ERROR: f64 = 2e-5;
/// The largest normal error accepted, in degrees.
const MAX_NORMAL_ERROR: f64 = 0.1;
/// The spacing of the sampled pixel centres, in pixels.
const STRIDE: usize = 2;

/// What one shape's sweep measured.
struct Sweep {
    points: usize,
    worst: f64,
    worst_at: [f64; 2],
    mean: f64,
    worst_angle: f64,
}

/// Evaluates the WGSL `sdf_sample` at every pixel centre whose exact
/// distance to `shape` lies within the band. `medial` says whether a
/// box-local point lies on the medial axis, where the nearest boundary
/// point is not unique and the normal is either of the tied ones.
fn sweep(shape: &Shape, medial: impl Fn([f64; 2]) -> bool) -> Sweep {
    let source = include_str!("../src/render/shared.wgsl");
    let module = naga::front::wgsl::parse_str(source).expect("shared.wgsl parses");
    let eval = Eval::new(&module);
    let sdf_sample = eval.function("sdf_sample");
    let (boxed, extra) = box_params(shape).expect("the shape is a box");
    let wgsl_shape = wgsl_shape(&boxed);
    let centre = extra * cherenkov::kurbo::Point::ORIGIN;
    let reach = [boxed.half[0] + BAND, boxed.half[1] + BAND];
    let mut sweep = Sweep {
        points: 0,
        worst: 0.0,
        worst_at: [0.0; 2],
        mean: 0.0,
        worst_angle: 0.0,
    };
    for py in pixel_centres(centre.y, reach[1]) {
        for px in pixel_centres(centre.x, reach[0]) {
            let (exact, exact_normal) = distance_and_normal(&boxed, &extra, [px, py]);
            if exact.abs() > BAND {
                continue;
            }
            let local = [px - centre.x, py - centre.y];
            let sample = eval.call(
                sdf_sample,
                vec![
                    wgsl_shape.clone(),
                    Value::vec(&[narrow(local[0]), narrow(local[1])]),
                    Value::Bool(false),
                ],
            );
            let Value::Struct(fields) = sample else {
                panic!("sdf_sample returns a DistanceSample");
            };
            let distance = f64::from(fields[0].components()[0]);
            let gradient = fields[1].components();
            let error = (distance - exact).abs();
            sweep.points += 1;
            sweep.mean += error;
            if error > sweep.worst {
                sweep.worst = error;
                sweep.worst_at = [px, py];
            }
            if !(exact < 0.0 && medial(local)) {
                let cos = f64::from(gradient[0])
                    .mul_add(exact_normal[0], f64::from(gradient[1]) * exact_normal[1]);
                sweep.worst_angle = sweep
                    .worst_angle
                    .max(cos.clamp(-1.0, 1.0).acos().to_degrees());
            }
        }
    }
    #[expect(clippy::cast_precision_loss, reason = "point counts are small")]
    let count = sweep.points as f64;
    sweep.mean /= count;
    sweep
}

/// Every [`STRIDE`]th pixel centre `k + 0.5` from the first one at or
/// past `centre` to `centre + reach`.
fn pixel_centres(centre: f64, reach: f64) -> impl Iterator<Item = f64> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the sweep spans a few hundred pixels"
    )]
    let (lo, hi) = ((centre - 0.5).ceil() as i32, (centre + reach).ceil() as i32);
    (lo..hi).step_by(STRIDE).map(|k| f64::from(k) + 0.5)
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "the shader evaluates in f32, as the engine does"
)]
const fn narrow(value: f64) -> f32 {
    value as f32
}

/// The oracle's box as the WGSL `Shape` struct.
fn wgsl_shape(boxed: &BoxShape) -> Value {
    Value::Struct(vec![
        Value::vec(&[narrow(boxed.half[0]), narrow(boxed.half[1])]),
        Value::Float(narrow(boxed.aspect)),
        Value::Float(narrow(boxed.exponent)),
        Value::vec(&boxed.radii.map(narrow)),
    ])
}

fn check(name: &str, sweep: &Sweep) {
    assert!(
        sweep.worst <= MAX_DISTANCE_ERROR && sweep.worst_angle <= MAX_NORMAL_ERROR,
        "{name}: {} points, distance error worst {:.3e} px at {:?}, mean {:.3e} px; \
         normal error worst {:.4} degrees",
        sweep.points,
        sweep.worst,
        sweep.worst_at,
        sweep.mean,
        sweep.worst_angle,
    );
}

#[test]
fn ellipse_distance_is_exact() {
    // The major axis is medial between the two evolute cusps.
    let (a, b) = (48.0_f64, 28.0_f64);
    let cusp = a.mul_add(a, -(b * b)) / a;
    let on_axis = |p: [f64; 2]| p[1].abs() < 1e-3 && p[0].abs() <= cusp;
    // Centred on a pixel corner, and on a pixel centre, so the major axis
    // is a row of samples.
    for centre in [72.0, 72.5] {
        let shape = Shape::Ellipse(Ellipse::new((centre, centre), (a, b), 0.0));
        check(&format!("ellipse at {centre}"), &sweep(&shape, on_axis));
    }
}

#[test]
fn continuous_corner_distance_is_exact() {
    // Inside, the corner diagonals and the straight part's bisectors are
    // medial; both are where the corner-local coordinates tie.
    let radius = 32.0;
    let rect = Rect::new(120.0, 120.0, 240.0, 232.0);
    let half = [rect.width() / 2.0, rect.height() / 2.0];
    let tied = |p: [f64; 2]| {
        let q = [p[0].abs() - half[0], p[1].abs() - half[1]];
        (q[0] - q[1]).abs() < 1e-3
    };
    for (smoothing, offset) in [(0.6, 0.0), (1.0, 0.5)] {
        let shape = Shape::Continuous(ContinuousRect::new(
            rect.with_origin((rect.x0 + offset, rect.y0 + offset)),
            radius,
            smoothing,
        ));
        check(
            &format!("continuous rect, smoothing {smoothing}"),
            &sweep(&shape, tied),
        );
    }
}
