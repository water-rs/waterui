//! Sparse polygon coverage on the 2^-16 pixel placement grid.
//!
//! Disjoint geometry integrates signed boundaries directly. Other geometry
//! resolves integer winding in horizontal strips before integrating area.
//! Column carry is integer strip height, never a floating-point prefix sum:
//! untouched exterior columns are exactly zero and full interiors exactly one.

mod storage;

use std::mem::MaybeUninit;
use std::ops::Range;
use storage::Table;

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

/// One row, with a bit per touched column. Samples are reset on first
/// write, carry on consumption; no draw clears its bounding-box area.
#[derive(Debug, Default)]
struct RowCells {
    cells: Table<Cell>,
    touched: Table<u64>,
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

    #[inline]
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
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        clippy::float_cmp,
        clippy::suboptimal_flops,
        reason = "columns are clamped to the surface; area is rounded once to f32"
    )]
    #[inline]
    fn boundary(&mut self, sign: i32, height: i32, xt: f64, xb: f64) {
        let (left, right) = (xt.min(xb), xt.max(xb));
        let end_px = (right / 65536.0).clamp(0.0, self.width as f64);
        let end = end_px as usize + usize::from(end_px > (end_px as usize) as f64);
        let start = (left / 65536.0).clamp(0.0, self.width as f64) as usize;
        let signed_height = sign * height;
        if right <= 0.0
            || left >= self.width as f64 * 65536.0
            || left == right && left == start as f64 * 65536.0
        {
            self.touch(end).cover += signed_height;
            return;
        }
        let end = end.max(start + usize::from(left == right));
        if end <= start + 1 && left >= start as f64 * 65536.0 && right <= end as f64 * 65536.0 {
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
#[derive(Clone, Copy, Debug)]
struct Segment {
    x0: i64,
    y0: i64,
    dx: i64,
    dy: i64,
    slope: f64,
    dir: i32,
    next: u32,
}

impl Segment {
    #[expect(
        clippy::cast_precision_loss,
        reason = "magnitude bounds the conversion error"
    )]
    fn magnitude(&self) -> f64 {
        (self.x0 as f64).abs() + (self.dx as f64).abs() + 1.0
    }

    #[inline]
    fn deposit(&self, cells: &mut RowCells, sign: i32, top: i32, bottom: i32) {
        if self.dx == 0 {
            cells.vertical(sign, bottom - top, self.x0);
        } else {
            cells.boundary(sign, bottom - top, self.at(top), self.at(bottom));
        }
    }

    /// Compare the original rational lines at a half-grid ordinate.
    /// Tile pieces bound both deltas; placement cancels in the difference.
    fn compare(self, other: Self, twice_y: i64) -> std::cmp::Ordering {
        self.position(other, twice_y)
            .then_with(|| self.slope_order(other))
    }

    fn slope_order(self, other: Self) -> std::cmp::Ordering {
        (i128::from(self.dx) * i128::from(other.dy))
            .cmp(&(i128::from(other.dx) * i128::from(self.dy)))
    }

    fn position(self, other: Self, twice_y: i64) -> std::cmp::Ordering {
        let ad = i128::from(self.dy);
        let bd = i128::from(other.dy);
        let ax = i128::from(self.dx);
        let bx = i128::from(other.dx);
        let distance = 2 * i128::from(self.x0 - other.x0) * ad * bd
            + i128::from(twice_y - 2 * self.y0) * ax * bd
            - i128::from(twice_y - 2 * other.y0) * bx * ad;
        distance.cmp(&0)
    }

    #[expect(
        clippy::cast_precision_loss,
        clippy::suboptimal_flops,
        reason = "placed coordinates and differences fit the f64 significand"
    )]
    fn at(self, y: i32) -> f64 {
        (i64::from(y) - self.y0) as f64 * self.slope + self.x0 as f64
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Piece {
    y0: i32,
    y1: i32,
    left: u16,
    right: u16,
    /// Signed (start + 1) of an open visible boundary; zero means hidden.
    opened: i32,
    position: f64,
    end_position: f64,
}

/// Certify the sign only when the forward-error intervals are disjoint.
/// An overlapping interval is decided by the exact integer determinant.
fn separated(left: f64, right: f64, error: f64) -> Option<std::cmp::Ordering> {
    let delta = left - right;
    if delta > error {
        Some(std::cmp::Ordering::Greater)
    } else if delta < -error {
        Some(std::cmp::Ordering::Less)
    } else {
        None
    }
}

impl Piece {
    /// Integrate only when visibility changes, not at unrelated strip events.
    fn visibility(&mut self, edge: &Segment, cells: &mut RowCells, sign: i32, y: i32, row: i32) {
        let previous = self.opened.signum();
        if previous != sign {
            if previous != 0 {
                edge.deposit(cells, previous, row + self.opened.abs() - 1, row + y);
            }
            self.opened = sign * (y + 1);
        }
    }
}

/// A row's exact grid events, sorted by consuming a three-level bitset.
/// Only occupied words are visited or cleared, regardless of the gaps.
#[derive(Debug, Default)]
struct RowEvents {
    words: Table<u64>,
    occupied: u32,
}

impl RowEvents {
    const LEAVES: usize = (1_usize << 16) / 64 + 1;
    const WORDS: usize = Self::LEAVES + Self::LEAVES.div_ceil(64);

    #[expect(
        clippy::cast_sign_loss,
        reason = "row event ordinates are in 0..=65536"
    )]
    fn insert(&mut self, y: i32) {
        let y = y as usize;
        let word = y / 64;
        self.words[word] |= 1 << (y % 64);
        self.words[Self::LEAVES + word / 64] |= 1 << (word % 64);
        self.occupied |= 1 << (word / 64);
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        reason = "row event ordinates fit i32"
    )]
    fn drain(&mut self, out: &mut Table<i32>) {
        out.clear();
        let mut occupied = std::mem::take(&mut self.occupied);
        while occupied != 0 {
            let group = occupied.trailing_zeros() as usize;
            occupied &= occupied - 1;
            let mut words = std::mem::take(&mut self.words[Self::LEAVES + group]);
            while words != 0 {
                let word = group * 64 + words.trailing_zeros() as usize;
                words &= words - 1;
                let mut bits = std::mem::take(&mut self.words[word]);
                while bits != 0 {
                    out.push((word * 64 + bits.trailing_zeros() as usize) as i32);
                    bits &= bits - 1;
                }
            }
        }
    }
}

/// All storage belongs to one raster worker and is reused across draws.
#[derive(Debug, Default)]
pub struct Compiler {
    storage: Vec<MaybeUninit<u64>>,
    width: usize,
    capacity: usize,
    height: usize,
    order_error: f64,
    edges: Table<Segment>,
    starts: Table<u32>,
    active: Table<usize>,
    pieces: Table<Piece>,
    columns: Table<[u32; 2]>,
    events: RowEvents,
    order: Table<usize>,
    breaks: Table<i32>,
    profile: Table<(i32, i32)>,
    next_profile: Table<(i32, i32)>,
    cells: RowCells,
}

impl Compiler {
    /// Reserves one allocation for the largest band selection in a pass.
    pub fn reserve(&mut self, width: usize, edges: usize, height: usize) {
        if width <= self.width
            && edges <= self.capacity
            && height <= self.height
            && !self.storage.is_empty()
        {
            return;
        }
        self.width = self.width.max(width);
        self.capacity = self.capacity.max(edges);
        self.height = self.height.max(height);
        let n = self.capacity;
        let row = self.width + 1;
        let words = |count: usize, size: usize| (count * size).div_ceil(8);
        let len = words(n, size_of::<Segment>())
            + words(self.height, size_of::<u32>())
            + words(n, size_of::<Piece>())
            + row
            + RowEvents::WORDS
            + 2 * words(n, size_of::<usize>())
            + words(4 * n + 2, size_of::<i32>())
            + 2 * words(2 * n + 1, size_of::<(i32, i32)>())
            + words(row, size_of::<Cell>())
            + row.div_ceil(64);
        self.storage.resize(len, MaybeUninit::uninit());
        let mut remaining = self.storage.as_mut_slice();
        // SAFETY: every table element has alignment at most u64.
        // bind partitions the live allocation into
        // disjoint tables. Only reserve resizes it, rebinding every table before
        // any subsequent use; no table reference survives this exclusive call.
        unsafe {
            self.edges = Table::bind(&mut remaining, n);
            self.starts = Table::bind(&mut remaining, self.height);
            self.pieces = Table::bind(&mut remaining, n);
            self.columns = Table::bind(&mut remaining, row);
            self.events.words = Table::bind(&mut remaining, RowEvents::WORDS);
            self.active = Table::bind(&mut remaining, n);
            self.order = Table::bind(&mut remaining, n);
            self.breaks = Table::bind(&mut remaining, 4 * n + 2);
            self.profile = Table::bind(&mut remaining, 2 * n + 1);
            self.next_profile = Table::bind(&mut remaining, 2 * n + 1);
            self.cells.cells = Table::bind(&mut remaining, row);
            self.cells.touched = Table::bind(&mut remaining, row.div_ceil(64));
        }
    }

    /// Read only the supplied band selection, never the source's other edges.
    #[expect(
        clippy::cast_possible_wrap,
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "surface dimensions fit u16; band-local ordinates fit i32"
    )]
    pub fn prepare<E: EdgeSource + ?Sized, S: fearless_simd::Simd>(
        &mut self,
        simd: S,
        source: &E,
        selection: &[u32],
        offset: Fixed,
        rows: Range<usize>,
        width: usize,
    ) {
        self.reserve(width, selection.len(), rows.len());
        self.edges.clear();
        self.pieces.clear();
        self.starts.resize(rows.len(), u32::MAX);
        self.active.clear();
        self.cells.resize(width);
        self.order_error = 0.0;
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
            let first_row = ((ya - top) / ONE) as usize;
            let next = self.starts[first_row];
            self.starts[first_row] = self.edges.len() as u32;
            let segment = Segment {
                x0,
                y0: y0 - top,
                dx: x1 - x0,
                dy: y1 - y0,
                slope: (x1 - x0) as f64 / (y1 - y0) as f64,
                dir,
                next,
            };
            self.order_error = self.order_error.max(segment.magnitude());
            self.edges.push(segment);
        });
        // For a point on a segment, |(y-y0)*dx/dy| <= |dx|. The four
        // integer-to-f64 conversions, division, multiplication and addition
        // incur less than 8u*(|x0|+|dx|+1), u=EPSILON/2. This pair bound
        // includes subtraction and rounding the bound itself. It determines
        // only separation, never coincidence.
        self.order_error *= 16.0 * f64::EPSILON;
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_possible_wrap,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "band rows and pixel units fit i32"
    )]
    pub fn rows(
        &mut self,
        height: usize,
        rule: FillRule,
        overlap: Overlap,
        mut emit: impl FnMut(usize, Range<usize>, Run<'_>),
    ) {
        if overlap == Overlap::Possible {
            self.pieces.resize(self.edges.len(), Piece::default());
            self.columns.resize(self.cells.width + 1, [u32::MAX; 2]);
            self.events.words.resize(RowEvents::WORDS, 0);
        }
        let mut row = 0;
        while row < height {
            let top = (row as i32) << 16;
            let bottom = top + ONE as i32;
            self.active
                .retain(|&i| self.edges[i].y0 + self.edges[i].dy > i64::from(top));
            let mut start = std::mem::replace(&mut self.starts[row], u32::MAX);
            while start != u32::MAX {
                self.active.push(start as usize);
                start = self.edges[start as usize].next;
            }
            let mut repeat_end = height;
            for &index in self.active.iter() {
                let e = &self.edges[index];
                let (a, b) = (
                    e.y0.max(i64::from(top)) as i32,
                    (e.y0 + e.dy).min(i64::from(bottom)) as i32,
                );
                // Vertical boundaries have identical integrals in every
                // complete row until an edge starts or ends. Shade those
                // rows from one compilation, without retaining a mask.
                if e.dx != 0 || a != top || b != bottom {
                    repeat_end = row + 1;
                } else {
                    repeat_end = repeat_end.min(((e.y0 + e.dy) >> 16) as usize);
                }
                if overlap == Overlap::Disjoint {
                    e.deposit(&mut self.cells, e.dir, a, b);
                } else {
                    let (xa, xb) = (e.at(a), e.at(b));
                    let left = (xa.min(xb) / 65536.0).clamp(0.0, self.cells.width as f64);
                    let right = (xa.max(xb) / 65536.0).clamp(0.0, self.cells.width as f64);
                    let end = right as u16;
                    self.pieces[index] = Piece {
                        y0: a - top,
                        y1: b - top,
                        left: left as u16,
                        right: end + u16::from(right > f64::from(end)),
                        opened: 0,
                        position: 0.0,
                        end_position: 0.0,
                    };
                }
            }
            if overlap == Overlap::Possible {
                self.ordered(rule, top);
            }
            let next = (row + 1..repeat_end)
                .find(|&next| self.starts[next] != u32::MAX)
                .unwrap_or(repeat_end);
            self.cells.read(overlap == Overlap::Disjoint, |range, run| {
                for y in row..next {
                    emit(y, range.clone(), run);
                }
            });
            row = next;
        }
    }

    fn ordered(&mut self, rule: FillRule, row: i32) {
        let mut first_column = self.cells.width;
        let mut last_column = 0;
        for &index in self.active.iter() {
            let piece = self.pieces[index];
            let column = usize::from(piece.left);
            // A zero-width boundary precedes the positive-width envelopes
            // starting here, so grouping is independent of insertion order.
            let bucket = usize::from(piece.left != piece.right);
            let head = &mut self.columns[column][bucket];
            // The start-row link has already been consumed for active edges.
            self.edges[index].next = *head;
            *head = u32::try_from(index).expect("compiler edge index fits u32");
            first_column = first_column.min(column);
            last_column = last_column.max(column);
        }
        self.active.clear();
        for column in first_column..=last_column {
            for bucket in 0..2 {
                let mut index = std::mem::replace(&mut self.columns[column][bucket], u32::MAX);
                while index != u32::MAX {
                    self.active.push(index as usize);
                    index = self.edges[index as usize].next;
                }
            }
        }
        self.profile.clear();
        self.profile.push((0, 0));
        let mut first = 0;
        while first < self.active.len() {
            let mut last = first + 1;
            let mut right = self.pieces[self.active[first]].right;
            // Pixel envelopes are half-open. Touching at an integer column
            // shares only the winding profile, not a cell's boundary area.
            while last < self.active.len() && self.pieces[self.active[last]].left < right {
                right = right.max(self.pieces[self.active[last]].right);
                last += 1;
            }
            let edge = self.edges[self.active[first]];
            let piece = self.pieces[self.active[first]];
            if last == first + 1
                && piece.y0 == 0
                && i64::from(piece.y1) == ONE
                && self.profile.len() == 1
            {
                let winding = &mut self.profile[0].1;
                let before = inside(*winding, rule);
                *winding += edge.dir;
                let after = inside(*winding, rule);
                if before != after {
                    edge.deposit(
                        &mut self.cells,
                        if after { 1 } else { -1 },
                        row,
                        row + piece.y1,
                    );
                }
            } else {
                self.group(first..last, rule, row);
            }
            first = last;
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::too_many_lines,
        reason = "one strip sweep with exact grid events and coincidence"
    )]
    fn group(&mut self, group: Range<usize>, rule: FillRule, row: i32) {
        for &(y, _) in self.profile.iter() {
            self.events.insert(y);
        }
        self.events.insert(ONE as i32);
        for &index in &self.active[group.clone()] {
            let piece = self.pieces[index];
            self.events.insert(piece.y0);
            self.events.insert(piece.y1);
        }
        self.events.drain(&mut self.breaks);
        self.next_profile.clear();
        self.order.clear();
        let mut profile = 0;
        let mut endpoint = 1;
        let mut top = 0;
        while endpoint < self.breaks.len() {
            let mut bottom = self.breaks[endpoint];
            if top == self.breaks[endpoint - 1] {
                self.order.retain(|&i| {
                    if self.pieces[i].y1 > top {
                        true
                    } else {
                        self.pieces[i].visibility(&self.edges[i], &mut self.cells, 0, top, row);
                        false
                    }
                });
                for &index in &self.active[group.clone()] {
                    if self.pieces[index].y0 == top {
                        self.order.push(index);
                    }
                }
            }
            // Crossings are floored to the placement grid. The order just
            // before top + 1 includes all crossings assigned to top, while a
            // crossing exactly at top + 1 still belongs to the next strip.
            let twice_after = 2 * (i64::from(row) + i64::from(top) + 1);
            for &index in self.order.iter() {
                self.pieces[index].position = self.edges[index].at(row + top + 1);
                self.pieces[index].end_position = self.edges[index].at(row + bottom);
            }
            let compare = |a: usize, b: usize| {
                separated(
                    self.pieces[a].position,
                    self.pieces[b].position,
                    self.order_error,
                )
                .unwrap_or_else(|| {
                    let (a, b) = (self.edges[a], self.edges[b]);
                    a.position(b, twice_after)
                        .then_with(|| b.slope_order(a))
                        .then_with(|| (a.x0, a.y0, a.dx, a.dy).cmp(&(b.x0, b.y0, b.dx, b.dy)))
                })
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

            // In an ordered active set, the earliest crossing is between
            // neighbors. Pairs with non-converging slopes cannot cross.
            for pair in self.order.windows(2) {
                if separated(
                    self.pieces[pair[0]].end_position,
                    self.pieces[pair[1]].end_position,
                    self.order_error,
                ) == Some(std::cmp::Ordering::Less)
                {
                    continue;
                }
                let (a, b) = (self.edges[pair[0]], self.edges[pair[1]]);
                let (ad, bd) = (i128::from(a.dy), i128::from(b.dy));
                let (ax, bx) = (i128::from(a.dx), i128::from(b.dx));
                let denominator = ax * bd - bx * ad;
                if denominator <= 0 {
                    continue;
                }
                let numerator = i128::from(b.x0 - a.x0) * ad * bd
                    + i128::from(a.y0 - i64::from(row)) * ax * bd
                    - i128::from(b.y0 - i64::from(row)) * bx * ad;
                if numerator >= i128::from(top + 1) * denominator
                    && numerator < i128::from(bottom) * denominator
                {
                    bottom = (numerator / denominator) as i32;
                }
            }

            while profile + 1 < self.profile.len() && self.profile[profile + 1].0 <= top {
                profile += 1;
            }
            let mut winding = self.profile[profile].1;
            let mut i = 0;
            while i < self.order.len() {
                let index = self.order[i];
                let edge = self.edges[index];
                let before = inside(winding, rule);
                winding += edge.dir;
                i += 1;
                while i < self.order.len()
                    && separated(
                        self.pieces[index].position,
                        self.pieces[self.order[i]].position,
                        self.order_error,
                    )
                    .is_none()
                    && edge.compare(self.edges[self.order[i]], twice_after).is_eq()
                {
                    let index = self.order[i];
                    winding += self.edges[index].dir;
                    self.pieces[index].visibility(&self.edges[index], &mut self.cells, 0, top, row);
                    i += 1;
                }
                let after = inside(winding, rule);
                self.pieces[index].visibility(
                    &edge,
                    &mut self.cells,
                    i32::from(after) - i32::from(before),
                    top,
                    row,
                );
            }
            if self.next_profile.last().is_none_or(|&(_, w)| w != winding) {
                self.next_profile.push((top, winding));
            }
            top = bottom;
            if top == self.breaks[endpoint] {
                endpoint += 1;
            }
        }
        for &index in self.order.iter() {
            self.pieces[index].visibility(&self.edges[index], &mut self.cells, 0, ONE as i32, row);
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
            compiler.reserve(width, 0, 0);
            let cells = &mut compiler.cells;
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
            self.0.cells.read(false, |range, run| {
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

    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "exercise rounded estimates against exact determinants"
    )]
    fn certified_positions_match_integer_order() {
        let mut seed = 0x2820_u64;
        let mut random = || {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            seed
        };
        for _ in 0..20_000 {
            let x0 = i64::try_from(random() >> 9).unwrap() - (1_i64 << 54);
            let dx = i64::try_from(random() >> 32).unwrap() - (1_i64 << 31);
            let dy = i64::try_from(random() >> 32).unwrap() + 1;
            let a = Segment {
                x0,
                y0: -(dy / 2),
                dx,
                dy,
                slope: dx as f64 / dy as f64,
                dir: 1,
                next: u32::MAX,
            };
            let shift = i64::try_from(random() % 17).unwrap() - 8;
            let delta = i64::try_from(random() >> 32).unwrap() - (1_i64 << 31);
            for (shift, delta) in [(0, 0), (shift, -2 * shift), (shift, delta), (delta, shift)] {
                let b = Segment {
                    x0: a.x0 + shift,
                    dx: a.dx + delta,
                    slope: (a.dx + delta) as f64 / dy as f64,
                    ..a
                };
                let error = a.magnitude().max(b.magnitude()) * (16.0 * f64::EPSILON);
                for y in [0, 1] {
                    if let Some(order) = separated(a.at(y), b.at(y), error) {
                        assert_eq!(order, a.position(b, 2 * i64::from(y)));
                    }
                }
            }
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
        let selection: Vec<_> = (0..u32::try_from(edges.len()).unwrap()).collect();
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
            f32::from((seed >> 24) as u8) / 32.0 - 2.0
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
