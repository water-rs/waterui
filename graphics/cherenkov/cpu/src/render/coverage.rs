//! Sparse polygon coverage on the 2^-16 pixel placement grid.
//!
//! Geometry has already resolved winding into visible boundary intervals.
//! Rasterization integrates those intervals without revisiting topology.
//! Column carry is integer strip height, never a floating-point prefix sum:
//! untouched exterior columns are exactly zero and full interiors exactly one.

mod storage;

use std::mem::MaybeUninit;
use std::ops::Range;
use storage::Table;

#[cfg(test)]
use cherenkov::FillRule;

#[cfg(test)]
use super::tiles::IntegerEdge;
use super::tiles::{EdgeTiles, Fixed, ONE};

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    Disjoint,
    Possible,
}

/// Coverage supplied to the compositing kernels, without materializing
/// constant interiors. Implementations are monomorphized in the span loops.
pub trait Source: Copy {
    fn check_len(self, len: usize);
    fn at(self, i: usize) -> f32;
    fn full(self) -> bool;
    fn slice(self, range: Range<usize>) -> Self;
}

impl Source for &[f32] {
    fn check_len(self, len: usize) {
        assert_eq!(self.len(), len, "composite coverage length mismatch");
    }
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
    fn check_len(self, _: usize) {}
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
    fn check_len(self, len: usize) {
        assert_eq!(self.0.len(), len, "composite coverage length mismatch");
    }
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

/// One row, with a bit per touched column. Consumption resets touched
/// cells; deposits need no first-write test or bounding-box clear.
#[derive(Debug, Default)]
struct RowCells {
    cells: Table<Cell>,
    touched: Table<u64>,
    width: usize,
    next: usize,
    changed: bool,
}

impl RowCells {
    fn resize(&mut self, width: usize) {
        self.width = width;
        self.cells
            .resize((width + 1).max(self.cells.len()), Cell::default());
        self.touched
            .resize((width + 1).div_ceil(64).max(self.touched.len()), 0);
        self.changed = false;
    }

    #[inline]
    fn touch(&mut self, x: usize) -> &mut Cell {
        let word = x / 64;
        let bit = 1u64 << (x % 64);
        self.touched[word] |= bit;
        &mut self.cells[x]
    }

    /// A vertical integer line has no slope or clipped ramp to integrate.
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "surface-clamped column; power-of-two scaling is exact and needs no FMA"
    )]
    #[inline]
    fn vertical(&mut self, sign: i32, height: i32, x: i64) {
        let x = x.clamp(0, (self.width as i64) << 16);
        let column = (x >> 16) as usize;
        let fraction = x & (ONE - 1);
        let height = sign * height;
        if fraction == 0 {
            self.touch(column).cover += height;
        } else {
            self.touch(column).partial +=
                (i64::from(height) * (ONE - fraction)) as f32 * (1.0 / 4_294_967_296.0);
            self.touch(column + 1).cover += height;
        }
    }

    /// Integral of the area to the right of a boundary. Only crossed
    /// columns receive fractions; every column beyond it receives exact
    /// integer height through one carry event.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "columns are clamped to the surface; area is rounded once to f32"
    )]
    #[expect(
        clippy::inline_always,
        reason = "keep the single-pixel path in the decoder; the multi-column integral stays out of line"
    )]
    #[inline(always)]
    fn boundary(&mut self, sign: i32, height: i32, xt: f64, xb: f64) {
        let (left, right) = (xt.min(xb), xt.max(xb));
        let signed_height = sign * height;
        if right <= 0.0 {
            self.touch(0).cover += signed_height;
            return;
        }
        if left >= self.width as f64 * 65536.0 {
            self.touch(self.width).cover += signed_height;
            return;
        }
        // Most retained intervals fit a single pixel. Their trapezoid
        // needs neither the general ramp nor an upper-column conversion.
        // SAFETY: max(left, 0) is finite and in [0, width*65536); the
        // surface width fits u16, so it fits usize on supported targets.
        let start = unsafe { left.max(0.0).to_int_unchecked::<usize>() } >> 16;
        if left >= 0.0 && right <= (start + 1) as f64 * 65536.0 {
            let twice_area =
                f64::from(signed_height) * (2.0 * (start as f64 + 1.0) * 65536.0 - xt - xb);
            self.touch(start).partial += twice_area as f32 * (1.0 / 8_589_934_592.0);
            self.touch(start + 1).cover += signed_height;
            return;
        }
        self.ramp(signed_height, left, right, start);
    }

    /// Only the two end columns need triangles. Interior samples form
    /// an arithmetic progression across the linear boundary.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "columns are surface-clamped; power-of-two scaling is exact"
    )]
    #[inline(never)]
    fn ramp(&mut self, signed_height: i32, left: f64, right: f64, start: usize) {
        let end_px = (right / 65536.0).min(self.width as f64);
        let end = end_px as usize + usize::from(end_px > (end_px as usize) as f64);
        let scale = f64::from(signed_height) / (65536.0 * (right - left));
        let triangle = scale * (0.5 / 65536.0);
        let mut first = start;
        if left >= 0.0 {
            let width = (start + 1) as f64 * 65536.0 - left;
            self.touch(start).partial += (width * width * triangle) as f32;
            first += 1;
        }
        let mut last = end;
        if right < end as f64 * 65536.0 {
            last -= 1;
            let width = right - last as f64 * 65536.0;
            self.touch(last).partial +=
                (f64::from(signed_height) / 65536.0 - width * width * triangle) as f32;
        }
        let mut area = ((first as f64 + 0.5) * 65536.0 - left) * scale;
        let step = 65536.0 * scale;
        for x in first..last {
            self.touch(x).partial += area as f32;
            area += step;
        }
        self.touch(end).cover += signed_height;
    }

    fn next(&mut self, word: &mut usize) -> Option<usize> {
        while *word < (self.width + 1).div_ceil(64) {
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
    fn read(&mut self, mut emit: impl FnMut(Range<usize>, Run<'_>)) {
        let fold = |v: f32| v.abs().min(1.0);
        let mut word = 0;
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
            for cell in &mut self.cells[start..end] {
                cell.partial = 0.0;
            }
            previous = end;
        }
        let constant = fold(cover as f32 / 65536.0);
        if previous < self.width && constant > 0.0 {
            emit(previous..self.width, Run::Uniform(Uniform(constant)));
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Vertical {
    x: i64,
    top: i64,
    bottom: i64,
    sign: i32,
}

/// A worker's sparse band rows. Sloped boundaries integrate on decode;
/// vertical intervals visit only rows where coverage changes.
#[derive(Debug, Default)]
pub struct Compiler {
    storage: Vec<MaybeUninit<u64>>,
    width: usize,
    height: usize,
    edges: usize,
    cells: Table<RowCells>,
    #[cfg(not(feature = "bounded-bins"))]
    verticals: Table<Vertical>,
}

impl Compiler {
    /// Allocated compiler scratch, including row cells and edge selection.
    pub const fn heap_bytes(&self) -> u64 {
        (self.storage.capacity() * size_of::<MaybeUninit<u64>>()) as u64
    }

    /// Reserves one allocation for a band or filter window.
    pub fn reserve(&mut self, width: usize, edges: usize, height: usize) {
        #[cfg(feature = "bounded-bins")]
        let edges = {
            let _ = edges;
            0
        };
        if width <= self.width
            && height <= self.height
            && edges <= self.edges
            && !self.storage.is_empty()
        {
            return;
        }
        self.width = self.width.max(width);
        self.height = self.height.max(height);
        self.edges = self.edges.max(edges);
        let row = self.width + 1;
        let words = |count: usize, size: usize| (count * size).div_ceil(8);
        let len = words(self.edges, size_of::<Vertical>())
            + words(self.height, size_of::<RowCells>())
            + self.height * (words(row, size_of::<Cell>()) + row.div_ceil(64));
        // Firmware reserves only the required rows, without Vec's geometric growth slack.
        #[cfg(feature = "bounded-bins")]
        self.storage.reserve_exact(len - self.storage.len());
        self.storage.resize(len, MaybeUninit::uninit());
        let mut remaining = self.storage.as_mut_slice();
        // SAFETY: every table element has alignment at most u64.
        // bind partitions the live allocation into
        // disjoint tables. Only reserve resizes it, rebinding every table before
        // any subsequent use; no table reference survives this exclusive call.
        // RowCells contains only non-owning tables and scalar state, so none
        // of the bound elements needs destruction before rebinding.
        unsafe {
            #[cfg(not(feature = "bounded-bins"))]
            {
                self.verticals = Table::bind(&mut remaining, self.edges);
            }
            self.cells = Table::bind(&mut remaining, self.height);
            for _ in 0..self.height {
                self.cells.push(RowCells {
                    cells: Table::bind(&mut remaining, row),
                    touched: Table::bind(&mut remaining, row.div_ceil(64)),
                    ..RowCells::default()
                });
            }
        }
    }

    /// Query only visible intervals intersecting the actual translated rows.
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::suboptimal_flops,
        reason = "surface dimensions fit u16; band-local ordinates fit i32"
    )]
    pub fn prepare(&mut self, source: &EdgeTiles, offset: Fixed, rows: Range<usize>, width: usize) {
        self.reserve(width, source.vertical_capacity(rows.len() > 16), rows.len());
        #[cfg(not(feature = "bounded-bins"))]
        self.verticals.clear();
        for cells in &mut self.cells[..rows.len()] {
            cells.resize(width);
        }
        let [ox, oy] = offset.units();
        let top = rows.start as i64 * ONE;
        let bottom = rows.end as i64 * ONE;
        source.visit(top - oy, bottom - oy, |edge| {
            let ya = (edge.top() + oy).max(top);
            let yb = (edge.bottom + oy).min(bottom);
            let dir = edge.sign();
            let x = edge.x + ox as f64;
            let mut row = ((ya - top) / ONE) as usize;
            let mut a = ya;
            if edge.slope == 0.0 {
                if x >= width as f64 * 65536.0 {
                    return;
                }
                #[cfg(not(feature = "bounded-bins"))]
                self.verticals.push(Vertical {
                    x: x as i64,
                    top: ya,
                    bottom: yb,
                    sign: dir,
                });
                for y in [ya, ya + ONE - 1, yb, yb + ONE - 1] {
                    let row = ((y - top) / ONE) as usize;
                    if row < rows.len() {
                        self.cells[row].changed = true;
                    }
                }
            } else {
                let mut xt = (ya - oy - edge.top()) as f64 * edge.slope + x;
                while a < yb {
                    let b = (top + (row + 1) as i64 * ONE).min(yb);
                    let xb = (b - a) as f64 * edge.slope + xt;
                    self.cells[row].changed = true;
                    self.cells[row].boundary(dir, (b - a) as i32, xt, xb);
                    row += 1;
                    a = b;
                    xt = xb;
                }
                if row < rows.len() {
                    self.cells[row].changed = true;
                }
            }
        });
        let mut next = rows.len();
        for row in (0..rows.len()).rev() {
            self.cells[row].next = next;
            if self.cells[row].changed {
                next = row;
            }
        }
        let mut deposit = |edge: &Vertical| {
            let mut row = ((edge.top - top) / ONE) as usize;
            let mut a = edge.top;
            while a < edge.bottom {
                let b = (top + (row + 1) as i64 * ONE).min(edge.bottom);
                self.cells[row].vertical(edge.sign, (b - a) as i32, edge.x);
                row = self.cells[row].next;
                a = top + row as i64 * ONE;
            }
        };
        #[cfg(not(feature = "bounded-bins"))]
        for edge in &*self.verticals {
            deposit(edge);
        }
        #[cfg(feature = "bounded-bins")]
        source.visit(top - oy, bottom - oy, |edge| {
            if edge.slope == 0.0 && edge.x + (ox as f64) < width as f64 * 65536.0 {
                deposit(&Vertical {
                    x: (edge.x + ox as f64) as i64,
                    top: (edge.top() + oy).max(top),
                    bottom: (edge.bottom + oy).min(bottom),
                    sign: edge.sign(),
                });
            }
        });
    }

    pub fn rows(&mut self, height: usize, mut emit: impl FnMut(usize, Range<usize>, Run<'_>)) {
        let mut row = 0;
        while row < height {
            let next = self.cells[row].next;
            self.cells[row].read(|range, run| {
                for y in row..next {
                    emit(y, range.clone(), run);
                }
            });
            row = next;
        }
    }
}

#[cfg(any(test, feature = "testing"))]
pub mod testing {
    use super::{Compiler, Run};

    /// A prepared sparse row; construction is outside the measured region.
    #[derive(Debug)]
    pub struct RowReader(Compiler);

    impl RowReader {
        /// Repeated fractional boundaries separated by constant interiors.
        #[must_use]
        #[expect(clippy::cast_precision_loss, reason = "benchmark row widths fit u16")]
        pub fn new(width: usize) -> Self {
            let mut compiler = Compiler::default();
            compiler.reserve(width, 0, 1);
            let cells = &mut compiler.cells[0];
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
            Self(compiler)
        }

        /// Consumes the row and returns its integrated area.
        #[expect(clippy::cast_precision_loss, reason = "benchmark run lengths fit u16")]
        pub fn read(&mut self) -> f32 {
            let mut sum = 0.0;
            self.0.cells[0].read(|range, run| {
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
    use cherenkov_oracle::metrics::assert_close;

    #[cfg(feature = "bounded-bins")]
    #[test]
    fn band_scratch_does_not_grow_with_boundary_count() {
        let mut compiler = Compiler::default();
        let mut bytes = None;
        for count in [2, 128, 8192] {
            let edges: Vec<_> = (0..count)
                .map(|i| {
                    let x = i * ONE / count;
                    IntegerEdge::new(x, 0, x, 16 * ONE)
                })
                .collect();
            let source = EdgeTiles::from_boundaries(&edges);
            compiler.prepare(&source, Fixed::new([0.25, 0.5]), 0..16, 64);
            let allocated = compiler.heap_bytes();
            assert_eq!(*bytes.get_or_insert(allocated), allocated);
            compiler.rows(16, |_, _, _| {});
        }
    }

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
        let raw: Vec<_> = edges.iter().copied().map(IntegerEdge::from).collect();
        let boundaries = if overlap == Overlap::Possible {
            super::super::topology::Sweep::default().boundaries(&raw, rule)
        } else {
            raw
        };
        let edges = EdgeTiles::from_boundaries(&boundaries);
        let mut out = vec![0.0; w * h];
        for top in (0..h).step_by(band) {
            let bottom = (top + band).min(h);
            compiler.prepare(&edges, Fixed::default(), top..bottom, w);
            compiler.rows(bottom - top, |row, range, run| {
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
                assert_close(&actual, expected);
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
            assert_close(&row[4..8], &[0.0; 4]);
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
            assert_close(&row[903..], &vec![0.0; row.len() - 903]);
        }
    }

    #[test]
    fn full_interiors_are_uniform() {
        let edges = polygon(&[(1.0, 0.0), (63.0, 0.0), (63.0, 1.0), (1.0, 1.0)]);
        let mut compiler = Compiler::default();
        compiler.prepare(
            &EdgeTiles::new(
                &mut super::super::tiles::EdgeScratch::default(),
                edges.into_iter(),
            ),
            Fixed::default(),
            0..1,
            64,
        );
        let mut uniform = 0;
        compiler.rows(1, |_, range, run| {
            if let Run::Uniform(cov) = run {
                assert_close(&[cov.0], &[1.0]);
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
            f32::from((seed >> 24) as u8) / 32.0 - 2.0
        };
        for points in (0..40)
            .map(|i| {
                (0..7 + i % 4 * 4)
                    .map(|_| (random(), random()))
                    .collect::<Vec<_>>()
            })
            .chain([vec![
                (1.0, 1.0),
                (7.0, 7.0),
                (1.0, 7.0),
                (7.0, 1.0),
                (4.0, 0.0),
                (4.0, 8.0),
                (0.0, 4.0),
                (8.0, 4.0),
            ]])
        {
            let edges = polygon(&points);
            // Horizontal edges still supply strip endpoints to the oracle,
            // even though the raster winding stream does not retain them.
            let segments: Vec<_> = points
                .iter()
                .zip(points.iter().cycle().skip(1))
                .map(|(&(x0, y0), &(x1, y1))| {
                    (f64::from(x0), f64::from(y0), f64::from(x1), f64::from(y1))
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
                        assert!((out[i] - split[i]).abs() <= 1e-5, "band split ({x},{y})");
                    }
                }
            }
        }
    }
}
