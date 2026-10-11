//! The CPU engine's box signed distance agrees with the WGSL `sdf_sample`
//! it transcribes.
//!
//! `RoundedBox::sdf_sample` is the one Rust copy of `shared.wgsl`'s box
//! distance eval; here it and the WGSL `sdf_sample` itself, running in
//! `cherenkov-shader`'s evaluator, read the same `f32` box at the same
//! `f32` box-local points — the adversarial set `gpu/tests/
//! corner_distance.rs` uses: pixel centres within 40 px of the boundary
//! in one closed quadrant, the axis lines included, plus points just off
//! an ellipse's major axis between its evolute cusps, where the
//! nearest-point equation has a near-triple root. The two must agree to
//! the same 1e-5 px and 0.1 degrees the WGSL solve holds against the
//! oracle's `f64` distance.

use cherenkov::kurbo::{Affine, Ellipse, Point, Rect};
use cherenkov::lowering::rounded_box::{BoxForm, RoundedBox, box_form};
use cherenkov::{ContinuousRect, ShapeData};
use cherenkov_oracle::sdf::{box_params, distance_and_normal};
use cherenkov_scene::{ContinuousRect as SceneContinuousRect, Shape};
use cherenkov_shader::eval::{Eval, Value};
use cherenkov_shader::naga::front::wgsl;

/// The band of signed distances checked, in pixels.
const BAND: f64 = 40.0;
/// The largest distance disagreement accepted, in pixels — the bound the
/// WGSL solve holds against the oracle.
const MAX_DISTANCE_ERROR: f64 = 1e-5;
/// The largest normal disagreement accepted, in degrees.
const MAX_NORMAL_ERROR: f64 = 0.1;
/// The spacing of the sampled pixel centres, in pixels.
const STRIDE: usize = 2;
/// Intervals the major axis is cut into, from the centre to one pixel past
/// the evolute cusp, for the points probed just off it.
const AXIS_PROBES: u16 = 96;
/// Distances from the major axis of the probed points, in pixels.
const AXIS_OFFSETS: [f64; 4] = [1e-3, 3e-4, 1e-4, 1e-5];

/// What one shape's sweep measured.
struct Sweep {
    points: usize,
    worst: f64,
    worst_at: [f64; 2],
    mean: f64,
    worst_angle: f64,
}

/// Evaluates the WGSL `sdf_sample` and the Rust `RoundedBox::sdf_sample` of
/// the engine's own box mapping of `engine` at every pixel centre whose
/// exact distance to `shape` lies within the band, and at the box-local
/// `probes`, and measures the disagreement between them. `medial` says
/// whether a box-local point lies on the medial axis, where the nearest
/// boundary point is not unique and the normal is either of the tied
/// ones.
fn sweep(
    shape: &Shape,
    engine: &ShapeData,
    probes: &[[f64; 2]],
    medial: impl Fn([f64; 2]) -> bool,
) -> Sweep {
    let module = wgsl::parse_str(include_str!("../../gpu/src/render/shared.wgsl"))
        .expect("shared.wgsl parses");
    let eval = Eval::new(&module);
    let sdf_sample = eval.function("sdf_sample");
    let (oracle_box, extra) = box_params(shape).expect("the shape is a box");
    let BoxForm::Box {
        extra: engine_extra,
        shape: boxed,
    } = box_form(engine)
    else {
        panic!("the shape maps to a box");
    };
    let centre = extra * Point::ORIGIN;
    assert_eq!(
        extra,
        Affine::translate(centre.to_vec2()),
        "the sweep runs on axis-aligned boxes"
    );
    assert_eq!(
        engine_extra, extra,
        "the engines' and the oracle's box mappings place the shape alike"
    );
    let wgsl_shape = wgsl_shape(&boxed);
    let reach = [oracle_box.half[0] + BAND, oracle_box.half[1] + BAND];
    let grid = pixel_centres(centre.y, reach[1]).flat_map(|py| {
        pixel_centres(centre.x, reach[0]).map(move |px| [px - centre.x, py - centre.y])
    });
    let mut sweep = Sweep {
        points: 0,
        worst: 0.0,
        worst_at: [0.0; 2],
        mean: 0.0,
        worst_angle: 0.0,
    };
    for local in grid.chain(probes.iter().copied()) {
        // Both evaluators see the point in `f32`; the oracle measures that
        // same point.
        let local = local.map(narrow);
        let (exact, _) = distance_and_normal(
            &oracle_box,
            &extra,
            [
                centre.x + f64::from(local[0]),
                centre.y + f64::from(local[1]),
            ],
        );
        if exact.abs() > BAND {
            continue;
        }
        let Value::Struct(fields) = eval.call(
            sdf_sample,
            vec![wgsl_shape.clone(), Value::vec(&local), Value::Bool(false)],
        ) else {
            panic!("sdf_sample returns a DistanceSample");
        };
        let wgsl_distance = f64::from(fields[0].components()[0]);
        let wgsl_gradient = fields[1].components();
        let (distance, nx, ny) = boxed.sdf_sample(local[0], local[1]);
        let error = (f64::from(distance) - wgsl_distance).abs();
        let local = local.map(f64::from);
        sweep.points += 1;
        sweep.mean += error;
        if error > sweep.worst {
            sweep.worst = error;
            sweep.worst_at = local;
        }
        if !(exact < 0.0 && medial(local)) {
            let cos = f64::from(nx).mul_add(
                f64::from(wgsl_gradient[0]),
                f64::from(ny) * f64::from(wgsl_gradient[1]),
            );
            sweep.worst_angle = sweep
                .worst_angle
                .max(cos.clamp(-1.0, 1.0).acos().to_degrees());
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
    reason = "the engine evaluates in f32"
)]
const fn narrow(value: f64) -> f32 {
    value as f32
}

/// The shared `RoundedBox` as the WGSL `Shape` struct.
fn wgsl_shape(boxed: &RoundedBox) -> Value {
    Value::Struct(vec![
        Value::vec(&boxed.half),
        Value::Float(boxed.aspect),
        Value::Float(boxed.exponent),
        Value::vec(&boxed.radii),
    ])
}

fn check(name: &str, sweep: &Sweep) {
    assert!(
        sweep.worst <= MAX_DISTANCE_ERROR && sweep.worst_angle <= MAX_NORMAL_ERROR,
        "{name}: {} points, distance disagreement worst {:.3e} px at box-local {:?}, \
         mean {:.3e} px; normal disagreement worst {:.4} degrees",
        sweep.points,
        sweep.worst,
        sweep.worst_at,
        sweep.mean,
        sweep.worst_angle,
    );
}

/// Checks the ellipse with semi-axes `radii`, the major one along x,
/// centred at `(centre, centre)`, at pixel centres and at points just off
/// its major axis between the evolute cusps.
fn check_ellipse(radii: [f64; 2], centre: f64) {
    let [a, b] = radii;
    // The major axis is medial between the two evolute cusps; with the
    // ellipse centred on a pixel centre it is a row of samples.
    let cusp = a.mul_add(a, -(b * b)) / a;
    let on_axis = move |p: [f64; 2]| p[1] == 0.0 && p[0].abs() <= cusp;
    let probes: Vec<[f64; 2]> = (0..=AXIS_PROBES)
        .flat_map(|k| {
            let along = (cusp + 1.0) * f64::from(k) / f64::from(AXIS_PROBES);
            AXIS_OFFSETS.map(move |off| [along, off])
        })
        .collect();
    let ellipse = Ellipse::new((centre, centre), (a, b), 0.0);
    check(
        &format!("{a}x{b} ellipse at {centre}"),
        &sweep(
            &Shape::Ellipse(ellipse),
            &ShapeData::Ellipse(ellipse),
            &probes,
            on_axis,
        ),
    );
}

#[test]
fn ellipse_distance_matches_the_wgsl() {
    // Centred on a pixel corner, and on a pixel centre.
    for centre in [72.0, 72.5] {
        check_ellipse([48.0, 28.0], centre);
    }
}

#[test]
fn eccentric_ellipse_distance_matches_the_wgsl_at_aspect_3() {
    check_ellipse([30.0, 10.0], 72.5);
}

#[test]
fn eccentric_ellipse_distance_matches_the_wgsl_at_aspect_10() {
    check_ellipse([40.0, 4.0], 72.5);
}

#[test]
fn eccentric_ellipse_distance_matches_the_wgsl_at_aspect_25() {
    check_ellipse([50.0, 2.0], 72.5);
}

#[test]
fn continuous_corner_distance_matches_the_wgsl() {
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
        let rect = rect.with_origin((rect.x0 + offset, rect.y0 + offset));
        check(
            &format!("continuous rect, smoothing {smoothing}"),
            &sweep(
                &Shape::Continuous(SceneContinuousRect::new(rect, radius, smoothing)),
                &ShapeData::Continuous(ContinuousRect::new(rect, radius).with_smoothing(smoothing)),
                &[],
                tied,
            ),
        );
    }
}
