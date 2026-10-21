//! Sparse polygon coverage on the 2^-16 pixel placement grid.
//!
//! Disjoint geometry integrates signed boundaries directly. Other geometry
//! resolves integer winding in horizontal strips before integrating area.
//! Column carry is integer strip height, never a floating-point prefix sum:
//! untouched exterior columns are exactly zero and full interiors exactly one.

use std::ops::Range;

use cherenkov::FillRule;

use super::tiles::{EdgeSource, Fixed, IntegerEdge, ONE};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    Disjoint,
    Possible,
}

impl Overlap {
    pub fn of(edges: &[super::raster::Edge]) -> Self {
        if cherenkov::lowering::proves_disjoint(edges) {
            Self::Disjoint
        } else {
            Self::Possible
        }
    }
}

/// Coverage supplied to the compositing kernels, without materializing
/// constant interiors. Implementations are monomorphized in the span loops.
pub trait Source: Copy {
    fn at(self, i: usize) -> f32;
    fn full(self) -> bool;
    fn slice(self, range: Range<usize>) -> Self;
}

impl Source for &[f32] {
    fn at(self, i: usize) -> f32 {
        self[i]
    }
    fn full(self) -> bool {
        false
    }
    fn slice(self, range: Range<usize>) -> Self {
        &self[range]
    }
}

#[derive(Clone, Copy)]
pub struct Uniform(pub f32);

impl Source for Uniform {
    fn at(self, _: usize) -> f32 {
        self.0
    }
    #[expect(
        clippy::float_cmp,
        reason = "exact full coverage enables direct stores"
    )]
    fn full(self) -> bool {
        self.0 == 1.0
    }
    fn slice(self, _: Range<usize>) -> Self {
        self
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Cell {
    partial: f32,
    cover: i32,
}

#[derive(Clone, Copy)]
pub struct Samples<'a>(&'a [Cell]);

impl Source for Samples<'_> {
    fn at(self, i: usize) -> f32 {
        self.0[i].partial
    }
    fn full(self) -> bool {
        false
    }
    fn slice(self, range: Range<usize>) -> Self {
        Self(&self.0[range])
    }
}

#[derive(Clone, Copy)]
pub enum Run<'a> {
    Uniform(Uniform),
    Samples(Samples<'a>),
    Slice(&'a [f32]),
}

impl Run<'_> {
    pub fn at(self, i: usize) -> f32 {
        match self {
            Self::Uniform(c) => c.at(i),
            Self::Samples(c) => c.at(i),
            Self::Slice(c) => c.at(i),
        }
    }
    pub fn slice(self, range: Range<usize>) -> Self {
        match self {
            Self::Uniform(c) => Self::Uniform(c),
            Self::Samples(c) => Self::Samples(c.slice(range)),
            Self::Slice(c) => Self::Slice(c.slice(range)),
        }
    }
}

/// One row, with a bit per touched column. Samples are reset on first
/// write, carry on consumption; no draw clears its bounding-box area.
#[derive(Debug, Default)]
struct RowCells {
    cells: Vec<Cell>,
    touched: Vec<u64>,
    first: usize,
    last: usize,
    width: usize,
}

impl RowCells {
    fn resize(&mut self, width: usize) {
        self.width = width;
        self.cells
            .resize((width + 1).max(self.cells.len()), Cell::default());
        self.touched
            .resize((width + 1).div_ceil(64).max(self.touched.len()), 0);
        self.first = usize::MAX;
        self.last = 0;
    }

    fn touch(&mut self, x: usize) -> &mut Cell {
        let word = x / 64;
        let bit = 1u64 << (x % 64);
        if self.touched[word] & bit == 0 {
            self.cells[x] = Cell::default();
            self.touched[word] |= bit;
            self.first = self.first.min(word);
            self.last = self.last.max(word);
        }
        &mut self.cells[x]
    }

    /// Integral of the area to the right of a boundary. Only crossed
    /// columns receive fractions; every column beyond it receives exact
    /// integer height through one carry event.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::float_cmp,
        clippy::suboptimal_flops,
        reason = "columns are clamped to the surface; area is rounded once to f32"
    )]
    fn boundary(&mut self, sign: i32, height: i32, xt: f64, xb: f64) {
        let (left, right) = (xt.min(xb), xt.max(xb));
        let end = (right / 65536.0).ceil().clamp(0.0, self.width as f64) as usize;
        let start = (left / 65536.0).floor().clamp(0.0, self.width as f64) as usize;
        let signed_height = sign * height;
        if right <= 0.0
            || left >= self.width as f64 * 65536.0
            || left == right && left % 65536.0 == 0.0
        {
            self.touch(end).cover += signed_height;
            return;
        }
        let end = end.max(start + usize::from(left == right));
        if end <= start + 1 {
            if start < self.width {
                let twice_area =
                    f64::from(signed_height) * (2.0 * (start as f64 + 1.0) * 65536.0 - xt - xb);
                self.touch(start).partial += twice_area as f32 * (1.0 / 8_589_934_592.0);
            }
        } else {
            let scale = f64::from(signed_height) / (65536.0 * (right - left));
            for x in start..end {
                let lo = (x as f64 * 65536.0).max(left);
                let hi = ((x as f64 + 1.0) * 65536.0).min(right);
                let ramp = (hi - lo) * ((lo - left) + (hi - left)) * 0.5;
                let full = ((x as f64 + 1.0) * 65536.0 - hi) * (right - left);
                self.touch(x).partial += ((ramp + full) * scale / 65536.0) as f32;
            }
        }
        self.touch(end).cover += signed_height;
    }

    fn next(&mut self, word: &mut usize) -> Option<usize> {
        while *word <= self.last {
            let bits = self.touched[*word];
            if bits != 0 {
                let bit = bits.trailing_zeros() as usize;
                self.touched[*word] = bits & (bits - 1);
                return Some(*word * 64 + bit);
            }
            *word += 1;
        }
        None
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "integer cover is at most a row height after winding resolution"
    )]
    fn read(&mut self, linear: bool, mut emit: impl FnMut(Range<usize>, Run<'_>)) {
        if self.first == usize::MAX {
            return;
        }
        let fold = |v: f32| {
            if linear {
                v.abs().min(1.0)
            } else {
                v.clamp(0.0, 1.0)
            }
        };
        let mut word = self.first;
        let mut next = self.next(&mut word);
        let mut cover = 0i32;
        let mut previous = 0;
        while let Some(start) = next {
            let constant = fold(cover as f32 / 65536.0);
            if previous < start && constant > 0.0 {
                emit(
                    previous..start.min(self.width),
                    Run::Uniform(Uniform(constant)),
                );
            }
            let mut end = start;
            loop {
                cover += self.cells[end].cover;
                self.cells[end].cover = 0;
                self.cells[end].partial = fold(cover as f32 / 65536.0 + self.cells[end].partial);
                end += 1;
                next = self.next(&mut word);
                if next != Some(end) {
                    break;
                }
            }
            let mut x = start;
            while x < end.min(self.width) {
                while x < end.min(self.width) && self.cells[x].partial <= 0.0 {
                    x += 1;
                }
                let first = x;
                while x < end.min(self.width) && self.cells[x].partial > 0.0 {
                    x += 1;
                }
                if first < x {
                    emit(first..x, Run::Samples(Samples(&self.cells[first..x])));
                }
            }
            previous = end;
        }
        let constant = fold(cover as f32 / 65536.0);
        if previous < self.width && constant > 0.0 {
            emit(previous..self.width, Run::Uniform(Uniform(constant)));
        }
        self.first = usize::MAX;
        self.last = 0;
    }
}

/// The original integer line, rebased in y only. Retaining its slope
/// avoids introducing a new line at every band and row cut.
#[derive(Clone, Copy)]
struct Segment {
    x0: i64,
    y0: i64,
    dx: i64,
    dy: i64,
    slope: f64,
    dir: i32,
}

impl Segment {
    /// Compare the original rational lines at a half-grid ordinate.
    /// Tile pieces bound both deltas; placement cancels in the difference.
    fn compare(self, other: Self, twice_y: i64) -> std::cmp::Ordering {
        let ad = i128::from(self.dy);
        let bd = i128::from(other.dy);
        let ax = i128::from(self.dx);
        let bx = i128::from(other.dx);
        let distance = 2 * i128::from(self.x0 - other.x0) * ad * bd
            + i128::from(twice_y - 2 * self.y0) * ax * bd
            - i128::from(twice_y - 2 * other.y0) * bx * ad;
        distance.cmp(&0).then_with(|| (ax * bd).cmp(&(bx * ad)))
    }

    #[expect(
        clippy::cast_precision_loss,
        reason = "placed coordinates and differences fit the f64 significand"
    )]
    fn at(self, y: i32) -> f64 {
        ((i64::from(y) - self.y0) as f64).mul_add(self.slope, self.x0 as f64)
    }
}

#[derive(Clone, Copy)]
struct Piece {
    edge: usize,
    x0: f64,
    x1: f64,
    y0: i32,
    y1: i32,
}

impl Piece {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "pixel envelope of finite bounded geometry"
    )]
    fn left(self) -> i64 {
        (self.x0.min(self.x1) / 65536.0).floor() as i64
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "pixel envelope of finite bounded geometry"
    )]
    fn right(self) -> i64 {
        (self.x0.max(self.x1) / 65536.0).ceil() as i64
    }
}

/// All storage belongs to one raster worker and is reused across draws.
#[derive(Default)]
pub struct Compiler {
    edges: Vec<Segment>,
    starts: Vec<usize>,
    active: Vec<usize>,
    pieces: Vec<Piece>,
    order: Vec<usize>,
    breaks: Vec<i32>,
    profile: Vec<(i32, i32)>,
    next_profile: Vec<(i32, i32)>,
    cells: RowCells,
}

impl Compiler {
    /// Read only the supplied band selection, never the source's other edges.
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        reason = "surface dimensions fit u16; band-local ordinates fit i32"
    )]
    pub fn prepare<E: EdgeSource + ?Sized, S: fearless_simd::Simd>(
        &mut self,
        simd: S,
        source: &E,
        selection: &[usize],
        offset: Fixed,
        rows: Range<usize>,
        width: usize,
    ) {
        self.edges.clear();
        self.starts.clear();
        self.active.clear();
        self.cells.resize(width);
        let [ox, oy] = offset.units();
        let top = rows.start as i64 * ONE;
        let bottom = rows.end as i64 * ONE;
        source.selected(simd, selection, |IntegerEdge { x0, y0, x1, y1 }| {
            let (x0, y0, x1, y1, dir) = if y0 < y1 {
                (x0 + ox, y0 + oy, x1 + ox, y1 + oy, 1)
            } else {
                (x1 + ox, y1 + oy, x0 + ox, y0 + oy, -1)
            };
            let (ya, yb) = (y0.max(top), y1.min(bottom));
            if ya >= yb {
                return;
            }
            if x0.min(x1) >= width as i64 * ONE {
                return;
            }
            let (x0, x1) = if x0.max(x1) <= 0 { (0, 0) } else { (x0, x1) };
            self.edges.push(Segment {
                x0,
                y0: y0 - top,
                dx: x1 - x0,
                dy: y1 - y0,
                slope: (x1 - x0) as f64 / (y1 - y0) as f64,
                dir,
            });
        });
        self.starts.extend(0..self.edges.len());
        self.starts.sort_unstable_by_key(|&i| self.edges[i].y0);
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "band rows and pixel units fit i32"
    )]
    pub fn rows(
        &mut self,
        height: usize,
        rule: FillRule,
        overlap: Overlap,
        mut emit: impl FnMut(usize, Range<usize>, Run<'_>),
    ) {
        let mut start = 0;
        for row in 0..height {
            let top = (row as i32) << 16;
            let bottom = top + ONE as i32;
            self.active
                .retain(|&i| self.edges[i].y0 + self.edges[i].dy > i64::from(top));
            while start < self.starts.len() && self.edges[self.starts[start]].y0 < i64::from(bottom)
            {
                self.active.push(self.starts[start]);
                start += 1;
            }
            self.pieces.clear();
            for &index in &self.active {
                let e = self.edges[index];
                let (a, b) = (
                    e.y0.max(i64::from(top)) as i32,
                    (e.y0 + e.dy).min(i64::from(bottom)) as i32,
                );
                let (xa, xb) = (e.at(a), e.at(b));
                if overlap == Overlap::Disjoint {
                    self.cells.boundary(e.dir, b - a, xa, xb);
                } else {
                    self.pieces.push(Piece {
                        edge: index,
                        x0: xa,
                        x1: xb,
                        y0: a - top,
                        y1: b - top,
                    });
                }
            }
            if overlap == Overlap::Possible {
                self.ordered(rule, top);
            }
            self.cells.read(overlap == Overlap::Disjoint, |range, run| {
                emit(row, range, run);
            });
        }
    }

    fn ordered(&mut self, rule: FillRule, row: i32) {
        self.pieces.sort_unstable_by(|a, b| {
            a.left()
                .cmp(&b.left())
                .then_with(|| a.x0.total_cmp(&b.x0))
                .then_with(|| a.x1.total_cmp(&b.x1))
                .then(a.y0.cmp(&b.y0))
                .then(a.y1.cmp(&b.y1))
        });
        self.profile.clear();
        self.profile.push((0, 0));
        let mut first = 0;
        while first < self.pieces.len() {
            let mut last = first + 1;
            let mut right = self.pieces[first].right();
            while last < self.pieces.len() && self.pieces[last].left() <= right {
                right = right.max(self.pieces[last].right());
                last += 1;
            }
            self.group(first..last, rule, row);
            first = last;
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "one strip sweep; exact coincidence is a geometry identity, not a tolerance"
    )]
    fn group(&mut self, group: Range<usize>, rule: FillRule, row: i32) {
        self.breaks.clear();
        self.breaks.extend(self.profile.iter().map(|&(y, _)| y));
        self.breaks.push(ONE as i32);
        for i in group.clone() {
            let a = self.pieces[i];
            self.breaks.extend([a.y0, a.y1]);
            for b in &self.pieces[i + 1..group.end] {
                let (lo, hi) = (a.y0.max(b.y0), a.y1.min(b.y1));
                if lo >= hi || a.x0.min(a.x1) > b.x0.max(b.x1) || b.x0.min(b.x1) > a.x0.max(a.x1) {
                    continue;
                }
                let sa = self.edges[a.edge];
                let sb = self.edges[b.edge];
                let ad = i128::from(sa.dy);
                let bd = i128::from(sb.dy);
                let ax = i128::from(sa.dx);
                let bx = i128::from(sb.dx);
                let denominator = ax * bd - bx * ad;
                if denominator == 0 {
                    continue;
                }
                let numerator = i128::from(sb.x0 - sa.x0) * ad * bd
                    + i128::from(sa.y0 - i64::from(row)) * ax * bd
                    - i128::from(sb.y0 - i64::from(row)) * bx * ad;
                let crossing = if denominator < 0 {
                    (-numerator).div_euclid(-denominator)
                } else {
                    numerator.div_euclid(denominator)
                };
                if crossing > i128::from(lo) && crossing < i128::from(hi) {
                    self.breaks.push(crossing as i32);
                }
            }
        }
        self.breaks.sort_unstable();
        self.breaks.dedup();
        self.next_profile.clear();
        self.order.clear();
        let mut profile = 0;
        for strip in self.breaks.windows(2) {
            let (top, bottom) = (strip[0], strip[1]);
            while profile + 1 < self.profile.len() && self.profile[profile + 1].0 <= top {
                profile += 1;
            }
            let mut winding = self.profile[profile].1;
            self.order.retain(|&i| self.pieces[i].y1 > top);
            for i in group.clone() {
                let p = self.pieces[i];
                if p.y0 == top {
                    self.order.push(i);
                }
            }
            let twice_middle = 2 * i64::from(row) + i64::from(top) + i64::from(bottom);
            let compare = |a: usize, b: usize| {
                let (a, b) = (self.pieces[a], self.pieces[b]);
                let (a, b) = (self.edges[a.edge], self.edges[b.edge]);
                a.compare(b, twice_middle)
                    .then_with(|| (a.x0, a.y0, a.dx, a.dy).cmp(&(b.x0, b.y0, b.dx, b.dy)))
            };
            for i in 1..self.order.len() {
                let value = self.order[i];
                let mut j = i;
                while j > 0 && compare(value, self.order[j - 1]).is_lt() {
                    self.order[j] = self.order[j - 1];
                    j -= 1;
                }
                self.order[j] = value;
            }
            let mut i = 0;
            while i < self.order.len() {
                let p = self.pieces[self.order[i]];
                let edge = self.edges[p.edge];
                let (xa, xb) = (edge.at(row + top), edge.at(row + bottom));
                let before = inside(winding, rule);
                winding += self.edges[p.edge].dir;
                i += 1;
                while i < self.order.len() {
                    let p = self.pieces[self.order[i]];
                    if !edge.compare(self.edges[p.edge], twice_middle).is_eq() {
                        break;
                    }
                    winding += self.edges[p.edge].dir;
                    i += 1;
                }
                let after = inside(winding, rule);
                if before != after {
                    self.cells
                        .boundary(if after { 1 } else { -1 }, bottom - top, xa, xb);
                }
            }
            if self.next_profile.last().is_none_or(|&(_, w)| w != winding) {
                self.next_profile.push((top, winding));
            }
        }
        std::mem::swap(&mut self.profile, &mut self.next_profile);
    }
}

const fn inside(winding: i32, rule: FillRule) -> bool {
    match rule {
        FillRule::NonZero => winding != 0,
        FillRule::EvenOdd => winding & 1 != 0,
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::{RowCells, Run};

    /// A prepared sparse row; construction is outside the measured region.
    #[derive(Debug)]
    pub struct RowReader(RowCells);

    impl RowReader {
        /// Repeated fractional boundaries separated by constant interiors.
        #[must_use]
        #[expect(clippy::cast_precision_loss, reason = "benchmark row widths fit u16")]
        pub fn new(width: usize) -> Self {
            let mut cells = RowCells::default();
            cells.resize(width);
            for x in (0..width).step_by(64) {
                cells.boundary(
                    1,
                    65536,
                    (x as f64 + 0.25) * 65536.0,
                    (x as f64 + 0.75) * 65536.0,
                );
                cells.boundary(
                    -1,
                    65536,
                    ((x + 48).min(width) as f64) * 65536.0,
                    ((x + 48).min(width) as f64) * 65536.0,
                );
            }
            Self(cells)
        }

        /// Consumes the row and returns its integrated area.
        #[expect(clippy::cast_precision_loss, reason = "benchmark run lengths fit u16")]
        pub fn read(&mut self) -> f32 {
            let mut sum = 0.0;
            self.0.read(false, |range, run| {
                if let Run::Uniform(c) = run {
                    sum = (range.len() as f32).mul_add(c.0, sum);
                } else {
                    for i in 0..range.len() {
                        sum += run.at(i);
                    }
                }
            });
            sum
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::raster::Edge;
    use super::*;

    #[expect(clippy::float_cmp, reason = "drop exactly horizontal test edges")]
    fn polygon(points: &[(f32, f32)]) -> Vec<Edge> {
        points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .filter_map(|(&(x0, y0), &(x1, y1))| (y0 != y1).then_some(Edge { x0, y0, x1, y1 }))
            .collect()
    }

    fn render(
        edges: &[Edge],
        size: (usize, usize),
        rule: FillRule,
        overlap: Overlap,
        band: usize,
    ) -> Vec<f32> {
        let (w, h) = size;
        let mut compiler = Compiler::default();
        let selection: Vec<_> = (0..edges.len()).collect();
        let mut out = vec![0.0; w * h];
        for top in (0..h).step_by(band) {
            let bottom = (top + band).min(h);
            compiler.prepare(
                fearless_simd::Fallback::new(),
                edges,
                &selection,
                Fixed::default(),
                top..bottom,
                w,
            );
            compiler.rows(bottom - top, rule, overlap, |row, range, run| {
                for (i, x) in range.enumerate() {
                    out[(top + row) * w + x] = run.at(i);
                }
            });
        }
        out
    }

    #[test]
    fn fractional_contours_and_overlap_resolve_before_area() {
        let quarter = polygon(&[(0.25, 0.25), (0.75, 0.25), (0.75, 0.75), (0.25, 0.75)]);
        for overlap in [Overlap::Disjoint, Overlap::Possible] {
            assert_eq!(
                render(&quarter, (3, 2), FillRule::NonZero, overlap, 2),
                [0.25, 0.0, 0.0, 0.0, 0.0, 0.0]
            );
        }
        let doubled: Vec<_> = quarter
            .iter()
            .copied()
            .cycle()
            .take(quarter.len() * 2)
            .collect();
        assert_eq!(
            render(&doubled, (1, 1), FillRule::NonZero, Overlap::Possible, 1),
            [0.25]
        );
        assert_eq!(
            render(&doubled, (1, 1), FillRule::EvenOdd, Overlap::Possible, 1),
            [0.0]
        );
        let mut cancelled = quarter.clone();
        cancelled.extend(quarter.iter().map(|e| Edge {
            x0: e.x1,
            y0: e.y1,
            x1: e.x0,
            y1: e.y0,
        }));
        assert_eq!(
            render(&cancelled, (1, 1), FillRule::NonZero, Overlap::Possible, 1),
            [0.0]
        );
        let triangle = polygon(&[(0.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        assert_eq!(
            render(&triangle, (1, 1), FillRule::NonZero, Overlap::Possible, 1),
            [0.5]
        );
    }

    #[test]
    fn coincident_boundaries_are_permutation_invariant() {
        fn permute(edges: &mut [Edge], at: usize, rule: FillRule, expected: &[f32]) {
            if at == edges.len() {
                let actual = render(edges, (2, 2), rule, Overlap::Possible, 1);
                assert!(
                    actual
                        .iter()
                        .zip(expected)
                        .all(|(a, b)| a.to_bits() == b.to_bits())
                );
            } else {
                for i in at..edges.len() {
                    edges.swap(at, i);
                    permute(edges, at + 1, rule, expected);
                    edges.swap(at, i);
                }
            }
        }
        for points in [
            [(0.25, 0.125), (0.75, 0.125), (0.75, 0.875), (0.25, 0.875)],
            [(0.125, 0.0), (0.75, 0.0), (1.75, 1.5), (1.125, 1.5)],
        ] {
            let mut edges = polygon(&points);
            let first = edges[0];
            let (mx, my) = (first.x0.midpoint(first.x1), first.y0.midpoint(first.y1));
            edges.extend([
                Edge {
                    x1: mx,
                    y1: my,
                    ..first
                },
                Edge {
                    x0: mx,
                    y0: my,
                    ..first
                },
                edges[1],
            ]);
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let expected = render(&edges, (2, 2), rule, Overlap::Possible, 1);
                permute(&mut edges, 0, rule, &expected);
            }
        }
    }

    #[test]
    fn disjoint_paths_agree_without_horizontal_edges() {
        let mut edges = polygon(&[(0.125, 0.25), (3.5, 0.25), (3.5, 5.75), (0.125, 5.75)]);
        edges.extend(polygon(&[(8.25, 1.5), (12.75, 7.25), (8.25, 7.25)]));
        let a = render(&edges, (16, 8), FillRule::NonZero, Overlap::Disjoint, 3);
        let b = render(&edges, (16, 8), FillRule::NonZero, Overlap::Possible, 3);
        assert!(
            a.iter()
                .zip(&b)
                .all(|(a, b)| (a - b).abs() <= 1.0 / 1_048_576.0)
        );
        for row in a.as_chunks::<16>().0 {
            assert!(row[4..8].iter().all(|v| v.to_bits() == 0));
        }
    }

    #[test]
    fn diagonal_has_no_exterior_residue() {
        let edges = polygon(&[(1.25, 0.0), (9.25, 0.0), (901.5, 901.0), (893.5, 901.0)]);
        let out = render(
            &edges,
            (1024, 901),
            FillRule::NonZero,
            Overlap::Possible,
            16,
        );
        for row in out.as_chunks::<1024>().0 {
            assert!(row[903..].iter().all(|v| v.to_bits() == 0));
        }
    }

    #[test]
    fn full_interiors_are_uniform() {
        let edges = polygon(&[(1.0, 0.0), (63.0, 0.0), (63.0, 1.0), (1.0, 1.0)]);
        let mut compiler = Compiler::default();
        compiler.prepare(
            fearless_simd::Fallback::new(),
            edges.as_slice(),
            &[0, 1],
            Fixed::default(),
            0..1,
            64,
        );
        let mut uniform = 0;
        compiler.rows(1, FillRule::NonZero, Overlap::Disjoint, |_, range, run| {
            if let Run::Uniform(cov) = run {
                assert_eq!(cov.0.to_bits(), 1.0f32.to_bits());
                uniform += range.len();
            }
        });
        assert!(uniform >= 61);
    }

    #[test]
    fn random_polygons_agree_with_oracle_and_band_splits() {
        let mut seed = 0x2820_u32;
        let mut random = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f32::from((seed >> 24) as u8) / 32.0
        };
        for _ in 0..40 {
            let points: Vec<_> = (0..7).map(|_| (random(), random())).collect();
            let edges = polygon(&points);
            let segments: Vec<_> = edges
                .iter()
                .map(|e| {
                    (
                        f64::from(e.x0),
                        f64::from(e.y0),
                        f64::from(e.x1),
                        f64::from(e.y1),
                    )
                })
                .collect();
            for rule in [FillRule::NonZero, FillRule::EvenOdd] {
                let out = render(&edges, (8, 8), rule, Overlap::Possible, 8);
                let split = render(&edges, (8, 8), rule, Overlap::Possible, 3);
                for y in 0..8u8 {
                    for x in 0..8u8 {
                        let i = usize::from(y) * 8 + usize::from(x);
                        let expected = cherenkov_oracle::coverage::testing::pixel_area(
                            &segments,
                            [f64::from(x), f64::from(y)],
                            match rule {
                                FillRule::NonZero => cherenkov_scene::FillRule::NonZero,
                                FillRule::EvenOdd => cherenkov_scene::FillRule::EvenOdd,
                            },
                        );
                        assert!(
                            (f64::from(out[i]) - expected).abs() <= 1e-5,
                            "{points:?} {rule:?} ({x},{y}): {} != {expected}",
                            out[i]
                        );
                        assert_eq!(out[i].to_bits(), split[i].to_bits(), "band split ({x},{y})");
                    }
                }
            }
        }
    }
}
