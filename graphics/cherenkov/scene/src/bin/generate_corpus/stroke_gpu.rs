//! Stroked scenes for the compute stroker (water-rs/waterui#2544).
//!
//! Each scene is a single `stroke` of one path on the root layer, 512 × 384,
//! with at least `GPU_ROUTE_MIN_ELEMENTS / 2` (2,048) source elements and an
//! outline above the GPU route's floors, so the automatic route selects it
//! once strokes become candidates at that size. Every family has a P3-only
//! and an above-SDR-white variant, and [`super::log_route_inputs`] logs the
//! numbers each scene presents to route selection.
//!
//! Trigonometry goes through `libm`, so the generated tree is the same bytes
//! on every host.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use cherenkov_scene::kurbo::{Affine, BezPath, Cap, Join, Point, Stroke, Vec2};
use cherenkov_scene::{Color, Paint, Shape, StrokeStyle};

use super::{Corpus, hdr, log_route_inputs, p3};

const W: u32 = 512;
const H: u32 = 384;

/// The scenes' surface, for the route-selection log.
const SURFACE: (f64, f64) = (512.0, 384.0);

/// One family: its geometry and style, logged once, and a scene per variant.
fn family(
    corpus: &mut Corpus,
    clear: Color,
    variants: &[(&str, Color)],
    path: &BezPath,
    style: &StrokeStyle,
    transform: Affine,
) {
    log_route_inputs(
        variants[0].0,
        path,
        Some(&Stroke::from(style)),
        transform,
        SURFACE,
    );
    for &(name, ink) in variants {
        let path = path.clone();
        let style = style.clone();
        corpus.scene(name, W, H, clear, |l| {
            l.transform(transform);
            l.stroke(Shape::Path { path }, style, Paint::Solid(ink));
        });
    }
}

pub fn generate(corpus: &mut Corpus, clear: Color) {
    joins(corpus, clear);
    caps(corpus, clear);
    dashes(corpus, clear);
    dash_ties_and_dense(corpus, clear);
    curves(corpus, clear);
    hairlines(corpus, clear);
    transformed(corpus, clear);
}

// ---- Joins -------------------------------------------------------------

/// Turn angles are counted in quarter degrees, so the exact reversal is an
/// exact integer.
const REVERSAL: u32 = 720;

/// The join scenes' stroke width.
const JOINS_WIDTH: f64 = 2.0;

/// The turn angles of the join grid, in quarter degrees: four below the
/// join threshold `2·tol / width` (1.15° at [`JOINS_WIDTH`] and the
/// identity's `tol = 0.02`), every 2° from 2° to 178°, and three near and
/// at an exact reversal. 96 angles, one per cell of an 8 × 12 grid.
fn turn_angles() -> Vec<u32> {
    let mut angles = vec![1, 2, 3, 4];
    angles.extend((1..=89).map(|k| 8 * k));
    angles.extend([716, 718, REVERSAL]);
    angles
}

/// A zig-zag from `origin` whose every turn is `quarters / 4` degrees:
/// segments heading alternately `±turn/2` from +x, `pitch` apart along x
/// and as tall as that heading makes them, up to `amp` — past it the
/// segments keep the height and close up along x. At most `max` segments,
/// within `span`. At [`REVERSAL`] the polyline runs back and forth along
/// +x instead, `2·pitch` forward and `pitch` back, so every turn is an
/// exact reversal.
fn zigzag(
    p: &mut BezPath,
    origin: Point,
    quarters: u32,
    pitch: f64,
    amp: f64,
    span: f64,
    max: u32,
) {
    p.move_to(origin);
    if quarters == REVERSAL {
        let mut x = origin.x;
        for k in 0..max {
            x += if k % 2 == 0 { 2.0 * pitch } else { -pitch };
            p.line_to((x, origin.y));
        }
        return;
    }
    let (s, c) = libm::sincos((f64::from(quarters) / 8.0).to_radians());
    let (dx, dy) = if pitch * s <= amp * c {
        (pitch, pitch * s / c)
    } else {
        (amp * c / s, amp)
    };
    let mut at = origin;
    for k in 0..max {
        let next = at + Vec2::new(dx, if k % 2 == 0 { -dy } else { dy });
        if next.x > origin.x + span {
            break;
        }
        p.line_to(next);
        at = next;
    }
}

/// Two zig-zags per cell, one cell per [`turn_angles`] entry, each at most
/// 14 segments 6 px apart and 8 px tall.
fn join_grid() -> BezPath {
    let mut p = BezPath::new();
    for (i, theta) in (0u32..).zip(turn_angles()) {
        let x = f64::from(i % 8).mul_add(64.0, 4.0);
        let y = f64::from(i / 8) * 32.0;
        for row in [12.0, 28.0] {
            zigzag(&mut p, Point::new(x, y + row), theta, 6.0, 8.0, 56.0, 14);
        }
    }
    p
}

fn joins(corpus: &mut Corpus, clear: Color) {
    let grid = join_grid();
    for (stem, join, miter_limit) in [
        ("miter", Join::Miter, 4.0),
        ("bevel", Join::Bevel, 4.0),
        ("round", Join::Round, 4.0),
        // A stroke carries one limit: at 1.5 the turns above about 96°
        // exceed it and fall back to bevels.
        ("miter-lim15", Join::Miter, 1.5),
    ] {
        let p3_name = format!("stroke-gpu-joins-{stem}-p3");
        let hdr_name = format!("stroke-gpu-joins-{stem}-hdr");
        family(
            corpus,
            clear,
            &[
                (&p3_name, p3(0.0, 0.55, 1.0)),
                (&hdr_name, hdr(6.0, 2.5, 0.4)),
            ],
            &grid,
            &StrokeStyle {
                width: JOINS_WIDTH,
                join,
                miter_limit,
                ..StrokeStyle::default()
            },
            Affine::IDENTITY,
        );
    }
}

// ---- Caps --------------------------------------------------------------

/// An 8 × 16 grid of 64 × 24 cells, each holding an open 16-segment
/// polyline, an open cubic, a single-segment subpath, and two zero-length
/// subpaths (a `LineTo` and a `CurveTo` back onto their start point).
fn cap_grid() -> BezPath {
    let mut p = BezPath::new();
    for i in 0u32..128 {
        let o = Vec2::new(f64::from(i % 8) * 64.0, f64::from(i / 8) * 24.0);
        p.move_to(Point::new(4.0, 3.0) + o);
        for k in 1u32..=16 {
            let y = if k % 2 == 0 { 3.0 } else { 7.0 };
            p.line_to(Point::new(3.5f64.mul_add(f64::from(k), 4.0), y) + o);
        }
        p.move_to(Point::new(4.0, 20.0) + o);
        p.curve_to(
            Point::new(14.0, 10.0) + o,
            Point::new(24.0, 24.0) + o,
            Point::new(34.0, 14.0) + o,
        );
        p.move_to(Point::new(40.0, 13.0) + o);
        p.line_to(Point::new(52.0, 21.0) + o);
        let dot = Point::new(56.0, 17.0) + o;
        p.move_to(dot);
        p.line_to(dot);
        let dot = Point::new(60.0, 20.0) + o;
        p.move_to(dot);
        p.curve_to(dot, dot, dot);
    }
    p
}

fn caps(corpus: &mut Corpus, clear: Color) {
    let grid = cap_grid();
    for (stem, start_cap, end_cap) in [
        ("square-round", Cap::Square, Cap::Round),
        ("round-butt", Cap::Round, Cap::Butt),
        ("butt-square", Cap::Butt, Cap::Square),
    ] {
        let p3_name = format!("stroke-gpu-caps-{stem}-p3");
        let hdr_name = format!("stroke-gpu-caps-{stem}-hdr");
        family(
            corpus,
            clear,
            &[
                (&p3_name, p3(1.0, 0.0, 0.45)),
                (&hdr_name, hdr(0.4, 5.0, 1.5)),
            ],
            &grid,
            &StrokeStyle {
                width: 3.0,
                start_cap,
                end_cap,
                ..StrokeStyle::default()
            },
            Affine::IDENTITY,
        );
    }
}

// ---- Dashes ------------------------------------------------------------

/// The circle of `radius` about `centre` as four cubic quarter arcs, closed.
fn circle(path: &mut BezPath, centre: Point, radius: f64) {
    path.move_to(centre + Vec2::new(radius, 0.0));
    arc(path, centre, radius, 0.0, TAU, 4);
    path.close_path();
}

/// `count` cubic arcs of `radius` about `centre` from angle `start` through
/// `sweep`, continuing the current subpath (the first arc starts where the
/// current point is meant to be, `centre + radius·(cos start, sin start)`).
fn arc(path: &mut BezPath, centre: Point, radius: f64, start: f64, sweep: f64, count: u32) {
    let step = sweep / f64::from(count);
    let handle = 4.0 / 3.0 * libm::tan(step / 4.0) * radius;
    for i in 0..count {
        let (s0, c0) = libm::sincos(step.mul_add(f64::from(i), start));
        let (s1, c1) = libm::sincos(step.mul_add(f64::from(i + 1), start));
        let from = centre + Vec2::new(radius * c0, radius * s0);
        let to = centre + Vec2::new(radius * c1, radius * s1);
        path.curve_to(
            from + Vec2::new(-handle * s0, handle * c0),
            to - Vec2::new(-handle * s1, handle * c1),
            to,
        );
    }
}

/// A 16 × 12 grid of 32 × 32 cells, each holding a closed rectangle, a
/// closed circle inside it and an open cubic. The rectangle's side and the
/// circle's radius vary by cell, so some contours end inside a dash and
/// `kurbo::dash` merges that dash through `ClosePath`.
fn dash_grid() -> BezPath {
    let mut p = BezPath::new();
    for i in 0u32..192 {
        let o = Vec2::new(f64::from(i % 16) * 32.0, f64::from(i / 16) * 32.0);
        let inset = 3.0 + f64::from(i % 5);
        let far = 32.0 - inset;
        p.move_to(Point::new(inset, inset) + o);
        p.line_to(Point::new(far, inset) + o);
        p.line_to(Point::new(far, far) + o);
        p.line_to(Point::new(inset, far) + o);
        p.close_path();
        circle(
            &mut p,
            Point::new(16.0, 16.0) + o,
            0.75f64.mul_add(-f64::from(i % 4), 8.0),
        );
        p.move_to(Point::new(10.0, 13.0) + o);
        p.curve_to(
            Point::new(14.0, 7.0) + o,
            Point::new(18.0, 25.0) + o,
            Point::new(22.0, 19.0) + o,
        );
    }
    p
}

fn dashes(corpus: &mut Corpus, clear: Color) {
    let grid = dash_grid();
    let round = StrokeStyle {
        width: 1.5,
        start_cap: Cap::Round,
        end_cap: Cap::Round,
        ..StrokeStyle::default()
    };
    // An odd-length pattern repeats doubled, alternating which entries are
    // dashes; the offset starts inside the first dash.
    family(
        corpus,
        clear,
        &[
            ("stroke-gpu-dash-p3", p3(0.0, 0.7, 0.2)),
            ("stroke-gpu-dash-hdr", hdr(5.0, 0.6, 3.0)),
        ],
        &grid,
        &StrokeStyle {
            dash_pattern: vec![5.0, 2.0, 1.0],
            dash_offset: 2.5,
            ..round
        },
        Affine::IDENTITY,
    );
    // Zero-length dashes: SVG 2 draws both caps at the dash's point — a
    // disc for the round caps used here (a tangent square for square,
    // nothing for butt).
    family(
        corpus,
        clear,
        &[
            ("stroke-gpu-dash-zero-p3", p3(0.0, 0.7, 0.2)),
            ("stroke-gpu-dash-zero-hdr", hdr(5.0, 0.6, 3.0)),
        ],
        &grid,
        &StrokeStyle {
            dash_pattern: vec![0.0, 4.0],
            ..round
        },
        Affine::IDENTITY,
    );
}

/// The tie pattern: period 20, on-entries `[0, 10)` and `[15, 18)`.
const TIE_PATTERN: [f64; 4] = [10.0, 5.0, 3.0, 2.0];
/// The tie offset: every subpath starts exactly on the second on-entry.
const TIE_OFFSET: f64 = 15.0;

/// The closed integer rectangles of the tie grid, `(width, height)`, each
/// starting at its top-left corner in phase 15 and running clockwise:
/// - 20 × 20: every corner and the end land on phase 15, the start of an
///   on-entry, so dashes start exactly on corners and the contour ends
///   exactly where a dash would begin — no merge;
/// - 23 × 20: corners land on phase 18 (dash ends) and the end in a dash,
///   which merges through `ClosePath`;
/// - 25 × 20: corners land on phase 0 (dash starts after a gap), the end in
///   a dash, merged;
/// - 16 × 15: the end lands in a dash, merged, corners off the boundaries.
const TIE_RECTS: [(f64, f64); 4] = [(20.0, 20.0), (23.0, 20.0), (25.0, 20.0), (16.0, 15.0)];

/// A 16 × 16 grid of 32 × 24 cells. Even cells hold one closed
/// [`TIE_RECTS`] rectangle; odd cells hold two open axis-aligned polylines
/// whose segment lengths are the pattern's entries in order: a two-period
/// staircase (3 right, 2 down, 10 right, 5 down, twice) whose every corner
/// and end is a dash boundary, and a 3 + 2 + 10 polyline that ends exactly
/// on the end of a dash.
fn dash_tie_grid() -> BezPath {
    let mut p = BezPath::new();
    for i in 0u32..256 {
        let o = Vec2::new(f64::from(i % 16) * 32.0, f64::from(i / 16) * 24.0);
        if i % 2 == 0 {
            let (w, h) = TIE_RECTS[(i / 2 % 4) as usize];
            let (x0, y0) = (3.0, 2.0);
            p.move_to(Point::new(x0, y0) + o);
            p.line_to(Point::new(x0 + w, y0) + o);
            p.line_to(Point::new(x0 + w, y0 + h) + o);
            p.line_to(Point::new(x0, y0 + h) + o);
            p.close_path();
        } else {
            let mut at = Point::new(3.0, 3.0) + o;
            p.move_to(at);
            for _ in 0..2 {
                for step in [
                    Vec2::new(3.0, 0.0),
                    Vec2::new(0.0, 2.0),
                    Vec2::new(10.0, 0.0),
                    Vec2::new(0.0, 5.0),
                ] {
                    at += step;
                    p.line_to(at);
                }
            }
            let mut at = Point::new(3.0, 22.0) + o;
            p.move_to(at);
            for step in [
                Vec2::new(3.0, 0.0),
                Vec2::new(0.0, -2.0),
                Vec2::new(10.0, 0.0),
            ] {
                at += step;
                p.line_to(at);
            }
        }
    }
    p
}

/// A 64 × 48 grid of 7.8 × 7.6 cells, inset so every stroke stays on the
/// surface, each holding one open subpath shorter
/// than the dense pattern's period of 11: a line of length 3 to 10.5 through
/// the cell centre at its own angle, every third one a cubic of about the
/// same chord. 3,072 subpaths in all.
fn dash_dense_grid() -> BezPath {
    let mut p = BezPath::new();
    for i in 0u32..3072 {
        let centre = Point::new(
            f64::from(i % 64).mul_add(7.8, 6.0),
            f64::from(i / 64).mul_add(7.6, 6.0),
        );
        let half = f64::from(i * 37 % 76).mul_add(0.1, 3.0) / 2.0;
        let (sin, cos) = libm::sincos(f64::from(i * 53 % 360).to_radians());
        let half_chord = Vec2::new(cos * half, sin * half);
        p.move_to(centre - half_chord);
        if i % 3 == 0 {
            let bulge = Vec2::new(-half_chord.y, half_chord.x) * 0.4;
            p.curve_to(
                centre - half_chord * 0.3 + bulge,
                centre + half_chord * 0.3 - bulge,
                centre + half_chord,
            );
        } else {
            p.line_to(centre + half_chord);
        }
    }
    p
}

fn dash_ties_and_dense(corpus: &mut Corpus, clear: Color) {
    family(
        corpus,
        clear,
        &[
            ("stroke-gpu-dash-ties-p3", p3(0.0, 0.45, 0.9)),
            ("stroke-gpu-dash-ties-hdr", hdr(1.0, 4.0, 7.0)),
        ],
        &dash_tie_grid(),
        &StrokeStyle {
            width: 1.5,
            join: Join::Miter,
            start_cap: Cap::Butt,
            end_cap: Cap::Square,
            dash_pattern: TIE_PATTERN.to_vec(),
            dash_offset: TIE_OFFSET,
            ..StrokeStyle::default()
        },
        Affine::IDENTITY,
    );
    // A zero-length gap splits each 8-long dash into a 6 and a 2 that meet
    // at one point, each with its own round caps.
    family(
        corpus,
        clear,
        &[
            ("stroke-gpu-dash-dense-p3", p3(0.85, 0.0, 0.3)),
            ("stroke-gpu-dash-dense-hdr", hdr(7.0, 1.0, 2.0)),
        ],
        &dash_dense_grid(),
        &StrokeStyle {
            width: 1.2,
            start_cap: Cap::Round,
            end_cap: Cap::Round,
            dash_pattern: vec![6.0, 0.0, 2.0, 3.0],
            ..StrokeStyle::default()
        },
        Affine::IDENTITY,
    );
}

// ---- Curves ------------------------------------------------------------

/// The arc radius of the curve chain, below the stroke's half width once
/// the chain is scaled into its cell.
const CHAIN_ARC_RADIUS: f64 = 0.6;
/// The curve scenes' stroke width.
const CURVES_WIDTH: f64 = 1.2;

/// One subpath of the degenerate curves the stroker has to handle, each
/// starting where the previous ends: a looped cubic, a true cusp (control
/// polygon `(0,0),(1,1),(0,1),(1,0)`), a cubic with `p1 == p0`, a colinear
/// cubic that reverses (two interior cusps, kurbo's `do_linear` branch), a
/// 270° arc of radius [`CHAIN_ARC_RADIUS`], two quads, a second loop, a quad
/// with `p1 == p0`, and a second cusp.
fn curve_chain() -> BezPath {
    let mut p = BezPath::new();
    let mut at = Point::ORIGIN;
    p.move_to(at);
    let cubic = |p: &mut BezPath, at: &mut Point, a: Vec2, b: Vec2, end: Vec2| {
        p.curve_to(*at + a, *at + b, *at + end);
        *at += end;
    };
    cubic(
        &mut p,
        &mut at,
        Vec2::new(12.0, 8.0),
        Vec2::new(-4.0, 8.0),
        Vec2::new(8.0, 0.0),
    );
    cubic(
        &mut p,
        &mut at,
        Vec2::new(6.0, 6.0),
        Vec2::new(0.0, 6.0),
        Vec2::new(6.0, 0.0),
    );
    cubic(
        &mut p,
        &mut at,
        Vec2::ZERO,
        Vec2::new(3.0, 6.0),
        Vec2::new(6.0, 4.0),
    );
    let v = Vec2::new(-6.0, 6.0);
    cubic(&mut p, &mut at, v * 1.6, v * -0.6, v);
    let centre = at + Vec2::new(0.0, CHAIN_ARC_RADIUS);
    arc(&mut p, centre, CHAIN_ARC_RADIUS, -FRAC_PI_2, 1.5 * PI, 3);
    at = centre + Vec2::new(-CHAIN_ARC_RADIUS, 0.0);
    for dy in [6.0, -6.0] {
        p.quad_to(at + Vec2::new(3.0, dy), at + Vec2::new(6.0, 0.0));
        at += Vec2::new(6.0, 0.0);
    }
    cubic(
        &mut p,
        &mut at,
        Vec2::new(10.5, -7.0),
        Vec2::new(-3.5, -7.0),
        Vec2::new(7.0, 0.0),
    );
    p.quad_to(at, at + Vec2::new(4.0, 4.0));
    at += Vec2::new(4.0, 4.0);
    cubic(
        &mut p,
        &mut at,
        Vec2::new(5.0, -5.0),
        Vec2::new(0.0, -5.0),
        Vec2::new(5.0, 0.0),
    );
    p
}

/// A 16 × 12 grid of 32 × 32 cells, each holding [`curve_chain`] rotated by
/// its own angle and scaled uniformly to fit the cell inside a margin of
/// the half width plus one pixel.
fn curve_grid() -> BezPath {
    let chain = curve_chain();
    let mut p = BezPath::new();
    let margin = CURVES_WIDTH / 2.0 + 1.0;
    for i in 0u32..192 {
        let mut cell = chain.clone();
        cell.apply_affine(super::rotate(TAU * f64::from(i) / 192.0));
        let bounds = cell.control_box();
        let room = 2.0f64.mul_add(-margin, 32.0);
        let scale = (room / bounds.width()).min(room / bounds.height());
        assert!(
            scale * CHAIN_ARC_RADIUS < CURVES_WIDTH / 2.0,
            "the chain's arc must stay below the half width"
        );
        let o = Vec2::new(f64::from(i % 16) * 32.0, f64::from(i / 16) * 32.0);
        let fit = Affine::translate(o + Vec2::new(margin, margin))
            * Affine::scale(scale)
            * Affine::translate(-bounds.origin().to_vec2());
        cell.apply_affine(fit);
        p.extend(cell.iter());
    }
    p
}

fn curves(corpus: &mut Corpus, clear: Color) {
    family(
        corpus,
        clear,
        &[
            ("stroke-gpu-curves-p3", p3(0.9, 0.0, 1.0)),
            ("stroke-gpu-curves-hdr", hdr(3.0, 0.5, 6.0)),
        ],
        &curve_grid(),
        &StrokeStyle {
            width: CURVES_WIDTH,
            join: Join::Round,
            ..StrokeStyle::default()
        },
        Affine::IDENTITY,
    );
}

// ---- Hairlines and transformed strokes ---------------------------------

/// `rows` horizontal sine polylines of `per_row` segments and `cols`
/// vertical waves of `per_col` cubic humps, inside the scene minus a
/// 16 px border, in the scene's device space.
fn wave_field(rows: u32, per_row: u32, cols: u32, per_col: u32) -> BezPath {
    let (x0, y0, x1, y1): (f64, f64, f64, f64) = (16.0, 16.0, 496.0, 368.0);
    let mut path = BezPath::new();
    for row in 0..rows {
        let y = (y1 - y0).mul_add((f64::from(row) + 0.5) / f64::from(rows), y0);
        let phase = f64::from(row) * 0.7;
        for k in 0..=per_row {
            let t = f64::from(k) / f64::from(per_row);
            let at = Point::new(
                (x1 - x0).mul_add(t, x0),
                5.0f64.mul_add(libm::sin(TAU.mul_add(t * 4.0, phase)), y),
            );
            if k == 0 {
                path.move_to(at);
            } else {
                path.line_to(at);
            }
        }
    }
    let step = (y1 - y0) / f64::from(per_col);
    for col in 0..cols {
        let x = (x1 - x0).mul_add((f64::from(col) + 0.5) / f64::from(cols), x0);
        path.move_to((x, y0));
        for k in 0..per_col {
            let top = step.mul_add(f64::from(k), y0);
            let swing = if k % 2 == 0 { 5.0 } else { -5.0 };
            path.curve_to(
                (x + swing, step.mul_add(1.0 / 3.0, top)),
                (x + swing, step.mul_add(2.0 / 3.0, top)),
                (x, top + step),
            );
        }
    }
    path
}

fn hairlines(corpus: &mut Corpus, clear: Color) {
    let field = wave_field(26, 48, 24, 36);
    for (name, width, ink) in [
        ("stroke-gpu-hairline-w025-p3", 0.25, p3(0.0, 0.35, 1.0)),
        ("stroke-gpu-hairline-w005-hdr", 0.05, hdr(12.0, 4.0, 0.5)),
    ] {
        family(
            corpus,
            clear,
            &[(name, ink)],
            &field,
            &StrokeStyle {
                width,
                ..StrokeStyle::default()
            },
            Affine::IDENTITY,
        );
    }
}

/// The transformed scenes' root layer transform: a rotation, an anisotropic
/// scale and a shear, so `sigma_max != 1` and the stroke is skewed.
const SKEW: Affine = Affine::new([2.6, 1.1, -0.4, 0.7, 48.0, 32.0]);

fn transformed(corpus: &mut Corpus, clear: Color) {
    // Authored in device space and pulled back through the transform, so the
    // centrelines land on the surface and only the stroke is skewed.
    let mut field = wave_field(30, 40, 24, 36);
    field.apply_affine(SKEW.inverse());
    family(
        corpus,
        clear,
        &[
            ("stroke-gpu-transformed-p3", p3(1.0, 0.3, 0.0)),
            ("stroke-gpu-transformed-hdr", hdr(0.5, 3.0, 8.0)),
        ],
        &field,
        &StrokeStyle {
            width: 1.2,
            join: Join::Round,
            start_cap: Cap::Round,
            end_cap: Cap::Round,
            ..StrokeStyle::default()
        },
        SKEW,
    );
}
