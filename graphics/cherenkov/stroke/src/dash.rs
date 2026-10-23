//! Dashing ahead of [`crate::stroke`]'s outline expansion, with the SVG 2
//! meaning for zero-length dashes.
//!
//! A zero-length dash draws both of its caps at its point: a disc of the
//! stroke's half width for round caps, a square of side `width` centred on
//! the point and oriented along the path tangent there for square caps, and
//! nothing for butt caps. A dash on a vertex takes the tangent of the
//! segment that starts there, and one at the end of an open subpath the
//! incoming tangent. `kurbo::dash` leaves a dot only where f64 rounding in
//! its `DashIterator::step` leaves a roughly 1e-15 segment behind, so the
//! walk below cuts the dashes itself with the same arclength arithmetic and
//! subsegments as kurbo and reports the zero-length ones as [`Dot`]s.

use std::f64::consts::{FRAC_PI_2, PI};

use kurbo::{
    Affine, Arc, BezPath, Cap, CubicBez, Line, ParamCurve, ParamCurveArclen, PathEl, PathSeg,
    Point, QuadBez, Stroke, Vec2,
};

/// kurbo's dash arclength accuracy, which also bounds how far from a vertex
/// a zero-length dash still counts as on it.
const DASH_ACCURACY: f64 = 1e-6;

/// Cut `path` into dashes under `pattern` at `offset`: the non-zero dashes
/// as a path for the outline stroke, and the zero-length ones as [`Dot`]s.
pub fn dash(path: impl IntoIterator<Item = PathEl>, pattern: &[f64], offset: f64) -> Dashed {
    Dasher::new(pattern, offset).dash(path)
}

/// A zero-length dash: its point and the unit tangent of the path there.
#[derive(Clone, Copy, Debug)]
pub struct Dot {
    center: Point,
    tangent: Vec2,
}

impl Dot {
    fn at(point: Point, tangent: Vec2) -> Self {
        Self {
            center: point,
            tangent: tangent.normalize(),
        }
    }

    /// Append the union of the dash's two caps as one contour that winds
    /// like the outlines `kurbo::stroke` produces, so a non-zero fill rule
    /// unions it with them.
    ///
    /// The contour is the start cap's half shape behind the point joined to
    /// the end cap's half shape ahead of it, in the frame whose x axis is
    /// the tangent.
    pub fn append_caps(self, style: &Stroke, tolerance: f64, out: &mut BezPath) {
        if style.start_cap == Cap::Butt && style.end_cap == Cap::Butt {
            return;
        }
        let hw = 0.5 * style.width;
        let mut local = BezPath::new();
        local.move_to((0.0, -hw));
        // Behind the point, from (0, -hw) to (0, hw).
        cap_half(&mut local, style.start_cap, -hw, hw, tolerance);
        // Ahead of it, from (0, hw) back to (0, -hw).
        cap_half(&mut local, style.end_cap, hw, -hw, tolerance);
        local.close_path();
        // The frame is a reflection (v is the tangent turned clockwise), which
        // reverses the contour above into kurbo's winding.
        let (u, v) = (self.tangent, Vec2::new(self.tangent.y, -self.tangent.x));
        let frame = Affine::new([u.x, u.y, v.x, v.y, self.center.x, self.center.y]);
        out.extend(frame * local);
    }
}

/// One cap's half of a dot contour, continuing from the contour's current
/// point on the axis `x = 0` to `(0, to_y)`: behind the point when `x_extent`
/// is negative, ahead of it when positive. `x_extent` is how far the cap
/// reaches along the tangent, the half width for both round and square caps.
fn cap_half(local: &mut BezPath, cap: Cap, x_extent: f64, to_y: f64, tolerance: f64) {
    match cap {
        Cap::Butt => local.line_to((0.0, to_y)),
        Cap::Square => {
            local.line_to((x_extent, -to_y));
            local.line_to((x_extent, to_y));
            local.line_to((0.0, to_y));
        }
        Cap::Round => {
            // A clockwise half circle starting on the axis below the point
            // when behind it, above the point when ahead.
            let start = if x_extent < 0.0 {
                -FRAC_PI_2
            } else {
                FRAC_PI_2
            };
            let hw = x_extent.abs();
            local.extend(Arc::new((0.0, 0.0), (hw, hw), start, -PI, 0.0).append_iter(tolerance));
        }
    }
}

/// The result of dashing: the non-zero dashes as a path for the outline
/// stroke, and the zero-length ones.
pub struct Dashed {
    pub dashes: BezPath,
    pub dots: Vec<Dot>,
}

/// A dash being collected: its start and the elements after it.
struct OpenDash {
    start: Point,
    elements: Vec<PathEl>,
}

/// Position in the dash pattern.
#[derive(Clone, Copy)]
struct Phase {
    ix: usize,
    remaining: f64,
    active: bool,
}

struct Dasher<'a> {
    dashes: &'a [f64],
    init: Phase,
}

impl<'a> Dasher<'a> {
    fn new(dashes: &'a [f64], offset: f64) -> Self {
        // An odd pattern repeats with its second pass on and off swapped, so
        // its period is doubled, as in kurbo.
        let sum: f64 = dashes.iter().sum();
        let period = if dashes.len() % 2 == 1 {
            2.0 * sum
        } else {
            sum
        };
        let offset = offset.rem_euclid(period);
        let mut phase = Phase {
            ix: 0,
            remaining: dashes[0] - offset,
            active: true,
        };
        loop {
            if phase.remaining >= 0.0 {
                break;
            }
            phase.ix = (phase.ix + 1) % dashes.len();
            phase.remaining += dashes[phase.ix];
            phase.active = !phase.active;
        }
        Self {
            dashes,
            init: phase,
        }
    }

    fn dash(&self, path: impl IntoIterator<Item = PathEl>) -> Dashed {
        let mut out = Dashed {
            dashes: BezPath::new(),
            dots: Vec::new(),
        };
        let mut segs: Vec<(PathSeg, f64)> = Vec::new();
        let mut start = Point::ORIGIN;
        let mut last = Point::ORIGIN;
        for el in path {
            match el {
                PathEl::MoveTo(p) => {
                    self.subpath(&segs, false, &mut out);
                    segs.clear();
                    start = p;
                    last = p;
                }
                PathEl::LineTo(p) => {
                    push_seg(&mut segs, PathSeg::Line(Line::new(last, p)));
                    last = p;
                }
                PathEl::QuadTo(p1, p2) => {
                    push_seg(&mut segs, PathSeg::Quad(QuadBez::new(last, p1, p2)));
                    last = p2;
                }
                PathEl::CurveTo(p1, p2, p3) => {
                    push_seg(&mut segs, PathSeg::Cubic(CubicBez::new(last, p1, p2, p3)));
                    last = p3;
                }
                PathEl::ClosePath => {
                    if last != start {
                        push_seg(&mut segs, PathSeg::Line(Line::new(last, start)));
                    }
                    self.subpath(&segs, true, &mut out);
                    segs.clear();
                    last = start;
                }
            }
        }
        self.subpath(&segs, false, &mut out);
        out
    }

    /// Dash one subpath of non-degenerate segments.
    fn subpath(&self, segs: &[(PathSeg, f64)], closed: bool, out: &mut Dashed) {
        if segs.is_empty() {
            return;
        }
        let mut walk = Walk {
            dasher: self,
            phase: self.init,
            current: None,
            first: None,
            broke: false,
            pending_dot: false,
            out,
        };
        walk.run(segs, closed);
    }
}

/// Keep a segment that has length; a degenerate one has no tangent and
/// takes no dash.
fn push_seg(segs: &mut Vec<(PathSeg, f64)>, seg: PathSeg) {
    let len = seg.arclen(DASH_ACCURACY);
    if len > 0.0 {
        segs.push((seg, len));
    }
}

/// The tangents at a segment's two ends, robust to control points that
/// coincide with an end (kurbo's own `PathSeg::tangents` is crate-private).
fn tangents(seg: &PathSeg) -> (Vec2, Vec2) {
    const EPS: f64 = 1e-12;
    let first = |candidates: [Vec2; 3]| {
        candidates
            .into_iter()
            .find(|d| d.hypot2() > EPS)
            .unwrap_or(candidates[2])
    };
    match seg {
        PathSeg::Line(l) => {
            let d = l.p1 - l.p0;
            (d, d)
        }
        PathSeg::Quad(q) => {
            let chord = q.p2 - q.p0;
            (
                first([q.p1 - q.p0, chord, chord]),
                first([q.p2 - q.p1, chord, chord]),
            )
        }
        PathSeg::Cubic(c) => {
            let chord = c.p3 - c.p0;
            (
                first([c.p1 - c.p0, c.p2 - c.p0, chord]),
                first([c.p3 - c.p2, c.p3 - c.p1, chord]),
            )
        }
    }
}

const fn seg_to_el(seg: &PathSeg) -> PathEl {
    match seg {
        PathSeg::Line(l) => PathEl::LineTo(l.p1),
        PathSeg::Quad(q) => PathEl::QuadTo(q.p1, q.p2),
        PathSeg::Cubic(c) => PathEl::CurveTo(c.p1, c.p2, c.p3),
    }
}

struct Walk<'a, 'o> {
    dasher: &'a Dasher<'a>,
    phase: Phase,
    /// The dash being collected, if the pattern is on.
    current: Option<OpenDash>,
    /// The subpath's first dash when it started at the subpath's start, held
    /// back so a closed subpath can join it to the dash that wraps through
    /// the closing point.
    first: Option<OpenDash>,
    /// Whether any dash has ended in this subpath.
    broke: bool,
    /// A zero-length dash on the vertex that the next segment starts.
    pending_dot: bool,
    out: &'o mut Dashed,
}

impl Walk<'_, '_> {
    fn run(&mut self, segs: &[(PathSeg, f64)], closed: bool) {
        let dashes = self.dasher.dashes;
        if self.phase.active && dashes[self.phase.ix] == 0.0 {
            self.pending_dot = true;
        }
        for (ix, &(seg, len)) in segs.iter().enumerate() {
            self.flush_dot(&seg);
            let mut t = 0.0;
            let mut seg_remaining = len;
            loop {
                if self.phase.remaining < seg_remaining {
                    // The next transition is a dash transition inside the
                    // segment, found on the part not yet consumed.
                    let rest = seg.subsegment(t..1.0);
                    // A remainder at or below the arclength accuracy is a
                    // transition at the segment end; inverting the arclength
                    // of such a degenerate rest is 0/0.
                    let t1 = if seg_remaining <= DASH_ACCURACY {
                        1.0
                    } else {
                        rest.inv_arclen(self.phase.remaining, DASH_ACCURACY)
                    };
                    if self.phase.active && self.phase.remaining > 0.0 {
                        self.extend(seg_to_el(&rest.subsegment(0.0..t1)), rest.start());
                    }
                    if self.phase.active {
                        self.end_dash();
                    }
                    t = t1.mul_add(1.0 - t, t);
                    seg_remaining -= self.phase.remaining;
                    self.advance();
                    if self.phase.active && dashes[self.phase.ix] == 0.0 {
                        if seg_remaining <= DASH_ACCURACY && (closed || ix + 1 < segs.len()) {
                            self.pending_dot = true;
                        } else {
                            let at = rest.subsegment(t1..1.0);
                            self.out.dots.push(Dot::at(at.start(), tangents(&at).0));
                        }
                    }
                } else {
                    if self.phase.active {
                        self.extend(seg_to_el(&seg.subsegment(t..1.0)), seg.eval(t));
                    }
                    self.phase.remaining -= seg_remaining;
                    break;
                }
            }
        }
        self.finish(segs, closed);
    }

    /// Draw a pending vertex dot with `seg`'s start tangent.
    fn flush_dot(&mut self, seg: &PathSeg) {
        if self.pending_dot {
            self.pending_dot = false;
            self.out.dots.push(Dot::at(seg.start(), tangents(seg).0));
        }
    }

    /// Add `el` to the current dash, starting one at `from` if the pattern
    /// just turned on.
    fn extend(&mut self, el: PathEl, from: Point) {
        self.current
            .get_or_insert_with(|| OpenDash {
                start: from,
                elements: Vec::new(),
            })
            .elements
            .push(el);
    }

    /// The pattern turns off: the current dash, if it has any length, is
    /// complete.
    fn end_dash(&mut self) {
        let dash = self.current.take();
        if !self.broke && self.dasher.init.active {
            // The dash that began at the subpath's start.
            self.first = dash;
        } else if let Some(dash) = dash {
            emit(&mut self.out.dashes, &dash);
        }
        self.broke = true;
    }

    /// Move to the next entry of the pattern.
    const fn advance(&mut self) {
        let dashes = self.dasher.dashes;
        self.phase.ix = (self.phase.ix + 1) % dashes.len();
        self.phase.remaining = dashes[self.phase.ix];
        self.phase.active = !self.phase.active;
    }

    /// Close out the subpath at its end.
    fn finish(&mut self, segs: &[(PathSeg, f64)], closed: bool) {
        let dashes = self.dasher.dashes;
        // The zero-length dashes that follow the dash running out at the
        // subpath's end sit on it. Looking ahead leaves the current dash
        // open, so a closed subpath still joins it to its first.
        let mut at_end = self.pending_dot;
        let mut ahead = self.phase;
        for _ in 0..dashes.len() {
            if ahead.remaining > DASH_ACCURACY {
                break;
            }
            ahead.ix = (ahead.ix + 1) % dashes.len();
            ahead.remaining = dashes[ahead.ix];
            ahead.active = !ahead.active;
            if ahead.active && dashes[ahead.ix] == 0.0 {
                at_end = true;
            }
        }
        if at_end {
            let dot = if closed {
                let seg = &segs[0].0;
                Dot::at(seg.start(), tangents(seg).0)
            } else {
                let seg = &segs[segs.len() - 1].0;
                Dot::at(seg.end(), tangents(seg).1)
            };
            self.out.dots.push(dot);
        }
        self.pending_dot = false;

        let tail = self.current.take();
        match (self.first.take(), tail) {
            // The pattern never turned off: one dash around the whole
            // subpath.
            (None, Some(whole)) if closed && !self.broke => {
                emit(&mut self.out.dashes, &whole);
                self.out.dashes.close_path();
            }
            // The last dash runs through the closing point into the first.
            (Some(first), Some(mut last)) if closed => {
                last.elements.extend(first.elements);
                emit(&mut self.out.dashes, &last);
            }
            (first, last) => {
                for dash in first.iter().chain(last.iter()) {
                    emit(&mut self.out.dashes, dash);
                }
            }
        }
    }
}

fn emit(path: &mut BezPath, dash: &OpenDash) {
    path.move_to(dash.start);
    path.extend(dash.elements.iter().copied());
}

#[cfg(test)]
mod tests {
    use kurbo::{Join, Shape as _, StrokeOpts};

    use super::*;
    use crate::stroke;

    const TOL: f64 = 0.01;

    fn style(width: f64, cap: Cap, dashes: &[f64], offset: f64) -> Stroke {
        Stroke::new(width)
            .with_caps(cap)
            .with_join(Join::Miter)
            .with_dashes(offset, dashes.iter().copied())
    }

    fn line(x0: f64, x1: f64) -> BezPath {
        let mut p = BezPath::new();
        p.move_to((x0, 0.0));
        p.line_to((x1, 0.0));
        p
    }

    fn outline(path: &BezPath, style: &Stroke) -> BezPath {
        stroke(path.iter(), style, TOL)
    }

    /// The dots of `path` under `style`, ordered by position along x.
    fn dots(path: &BezPath, style: &Stroke) -> Vec<Dot> {
        let mut found = Dasher::new(&style.dash_pattern, style.dash_offset)
            .dash(path.iter())
            .dots;
        found.sort_by(|a, b| a.center.x.total_cmp(&b.center.x));
        found
    }

    fn assert_finite(path: &BezPath, what: &str) {
        for el in path.iter() {
            let pts: &[Point] = match &el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => std::slice::from_ref(p),
                PathEl::QuadTo(a, b) => &[*a, *b],
                PathEl::CurveTo(a, b, c) => &[*a, *b, *c],
                PathEl::ClosePath => &[],
            };
            assert!(
                pts.iter().all(|p| p.x.is_finite() && p.y.is_finite()),
                "{what}: non-finite point in {el:?}"
            );
        }
    }

    #[test]
    fn a_zero_gap_after_a_dash_ending_at_the_segment_end_stays_finite() {
        let mut path = BezPath::new();
        path.move_to((424.712_887_282_334_86, 60.877_578_710_412_24));
        path.line_to((429.687_112_717_665_1, 57.522_421_289_587_754));
        let dashed = dash(path.iter(), &[6.0, 0.0, 2.0, 3.0], 0.0);
        assert_finite(&dashed.dashes, "dashes");
        assert!(dashed.dots.iter().all(|d| d.center.is_finite()));
        // The line is about 6 long: one dash, covering all of it.
        let len = dashed
            .dashes
            .segments()
            .map(|s| s.arclen(1e-9))
            .sum::<f64>();
        assert!((len - 6.0).abs() < 1e-3, "{len}");
    }

    #[test]
    fn dashing_lines_of_near_dash_length_stays_finite() {
        for cap in [Cap::Butt, Cap::Round, Cap::Square] {
            for k in -40..=40 {
                let len = f64::from(k).mul_add(6.6e-16, 6.0);
                for d in 0..16 {
                    let a = f64::from(d) * PI / 8.0;
                    let from = Point::new(424.712_887_282_334_86, 60.877_578_710_412_24);
                    let to = from + Vec2::new(a.cos(), a.sin()) * len;
                    let mut path = BezPath::new();
                    path.move_to(from);
                    path.line_to(to);
                    let dashed = dash(path.iter(), &[6.0, 0.0, 2.0, 3.0], 0.0);
                    assert_finite(&dashed.dashes, "dashes");
                    assert!(dashed.dots.iter().all(|d| d.center.is_finite()));
                    let out = outline(&path, &style(1.0, cap, &[6.0, 0.0, 2.0, 3.0], 0.0));
                    assert_finite(&out, "outline");
                }
            }
        }
    }

    #[test]
    fn a_zero_dash_draws_a_disc_for_round_caps() {
        let s = style(2.0, Cap::Round, &[0.0, 4.0], 0.0);
        let out = outline(&line(0.0, 8.0), &s);
        // Dots at x = 0, 4 and 8, each a disc of radius 1.
        let area = out.area().abs();
        let expected = 3.0 * PI;
        assert!((area - expected).abs() < 0.02, "{area}");
        assert_eq!(dots(&line(0.0, 8.0), &s).len(), 3);
    }

    #[test]
    fn a_zero_dash_draws_a_square_oriented_along_the_tangent() {
        let s = style(2.0, Cap::Square, &[0.0, 5.0], 0.0);
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((3.0, 4.0));
        let out = outline(&path, &s);
        // Dots at arclength 0 and 5, each a 2 by 2 square turned to the line.
        assert!((out.area().abs() - 8.0).abs() < 1e-9);
        let bbox = out.bounding_box();
        let c = (3.0f64 / 5.0, 4.0f64 / 5.0);
        let reach = c.0 + c.1;
        assert!((bbox.x0 + reach).abs() < 1e-9 && (bbox.x1 - 3.0 - reach).abs() < 1e-9);
    }

    #[test]
    fn mixed_caps_join_a_half_disc_to_a_half_square() {
        let mut s = style(2.0, Cap::Round, &[0.0, 9.0], 0.0);
        s.end_cap = Cap::Square;
        let area = outline(&line(0.0, 4.0), &s).area().abs();
        assert!(
            (area - (std::f64::consts::FRAC_PI_2 + 2.0)).abs() < 0.01,
            "{area}"
        );
    }

    #[test]
    fn butt_caps_draw_nothing_for_a_zero_dash() {
        let s = style(2.0, Cap::Butt, &[0.0, 4.0], 0.0);
        assert!(outline(&line(0.0, 8.0), &s).is_empty());
    }

    #[test]
    fn a_dot_winds_like_kurbo_outlines() {
        let dot = style(2.0, Cap::Round, &[0.0, 4.0], 0.0);
        let plain = style(2.0, Cap::Round, &[1.0, 3.0], 0.0);
        let dot_area = outline(&line(0.0, 3.0), &dot).area();
        let dash_area = outline(&line(0.0, 3.0), &plain).area();
        assert_eq!(dot_area.signum().to_bits(), dash_area.signum().to_bits());
    }

    #[test]
    fn a_dot_on_a_vertex_takes_the_outgoing_tangent() {
        let s = style(2.0, Cap::Square, &[0.0, 3.0], 0.0);
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((3.0, 0.0));
        path.line_to((3.0, 3.0));
        let found = dots(&path, &s);
        let vertex = found
            .iter()
            .find(|d| (d.center - Point::new(3.0, 0.0)).hypot() < 1e-9)
            .expect("a dot on the vertex");
        assert!((vertex.tangent - Vec2::new(0.0, 1.0)).hypot() < 1e-9);
    }

    #[test]
    fn a_dot_at_the_end_of_an_open_subpath_takes_the_incoming_tangent() {
        let s = style(2.0, Cap::Square, &[0.0, 3.0], 0.0);
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.line_to((3.0, 0.0));
        path.line_to((3.0, 3.0));
        let found = dots(&path, &s);
        let end = found.last().expect("dots");
        assert!((end.center - Point::new(3.0, 3.0)).hypot() < 1e-9);
        assert!((end.tangent - Vec2::new(0.0, 1.0)).hypot() < 1e-9);
    }

    #[test]
    fn a_closed_subpath_has_a_dot_where_it_starts_and_ends() {
        let s = style(2.0, Cap::Round, &[0.0, 4.0], 0.0);
        let square = kurbo::Rect::new(0.0, 0.0, 4.0, 4.0).to_path(TOL);
        let found = dots(&square, &s);
        let corners: Vec<_> = found.iter().map(|d| d.center).collect();
        assert!(
            corners
                .iter()
                .any(|c| (*c - Point::new(0.0, 0.0)).hypot() < 1e-9)
        );
    }

    /// The outline's subpaths as point lists, ordered by their first point:
    /// kurbo emits a closed path's dashes in a different order.
    fn subpaths(path: &BezPath) -> Vec<Vec<Point>> {
        let mut found: Vec<Vec<Point>> = Vec::new();
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(p) => found.push(vec![p]),
                PathEl::LineTo(p) => found.last_mut().expect("moveto").push(p),
                PathEl::QuadTo(a, b) => found.last_mut().expect("moveto").extend([a, b]),
                PathEl::CurveTo(a, b, c) => found.last_mut().expect("moveto").extend([a, b, c]),
                PathEl::ClosePath => {}
            }
        }
        found.sort_by(|a, b| a[0].x.total_cmp(&b[0].x).then(a[0].y.total_cmp(&b[0].y)));
        found
    }

    #[test]
    fn non_zero_dashes_match_kurbo() {
        let s = style(3.0, Cap::Round, &[5.0, 2.5, 1.0, 3.0], 1.7);
        let mut path = BezPath::new();
        path.move_to((0.0, 0.0));
        path.curve_to((20.0, 0.0), (20.0, 20.0), (0.0, 20.0));
        path.quad_to((-8.0, 10.0), (0.0, 0.0));
        path.close_path();
        path.move_to((40.0, 0.0));
        path.line_to((70.0, 5.0));
        let ours = outline(&path, &s);
        let theirs = kurbo::stroke(path.iter(), &s, &StrokeOpts::default(), TOL);
        let (ours, theirs) = (subpaths(&ours), subpaths(&theirs));
        assert_eq!(ours.len(), theirs.len());
        for (a, b) in ours.iter().zip(&theirs) {
            assert_eq!(a.len(), b.len());
            for (p, q) in a.iter().zip(b) {
                assert!((*p - *q).hypot() < 1e-9, "{p:?} {q:?}");
            }
        }
    }
}
