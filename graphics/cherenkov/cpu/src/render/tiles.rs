//! Quantized geometry compiled into normalized, directly indexed visible intervals.

mod intervals;
pub use intervals::Boundary;

use std::sync::Arc;

use super::raster::Edge;

/// The allocation unit of the retained layout: eight bytes.
pub type Word = u64;

/// A tile's side in pixels.
const TILE: f64 = 127.0;
/// Quantization units per pixel: one unit is 1/256 px and an `i16`
/// reaches ±128 px from its tile's origin.
const UNIT: f64 = 256.0;
/// A tile's side in units of 1/256 px.
const TILE_UNITS: i32 = 127 * 256;
/// Placement units per pixel.
pub const ONE: i64 = 1 << 16;

/// A device translation on the 2^-16 px grid. Splitting whole and
/// fractional parts keeps a pair in twelve bytes, including negative values.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fixed {
    whole: [i32; 2],
    fraction: [u16; 2],
}

impl Fixed {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "rounding to the placement grid; low bits are an unsigned fraction"
    )]
    pub fn new(pixels: [f64; 2]) -> Self {
        let units = pixels.map(|v| (v * 65536.0).round() as i64);
        Self {
            whole: units.map(|v| i32::try_from(v >> 16).expect("placement fits i32 pixels")),
            fraction: units.map(|v| v as u16),
        }
    }

    pub fn units(self) -> [i64; 2] {
        [0, 1].map(|i| (i64::from(self.whole[i]) << 16) + i64::from(self.fraction[i]))
    }
}

/// An edge in placement units. Widening precedes the shift so every
/// representable retained tile can be placed without losing low bits.
#[derive(Clone, Copy, Debug)]
pub struct IntegerEdge {
    pub x0: i64,
    pub y0: i64,
    pub x1: i64,
    pub y1: i64,
    pub top: i64,
    pub bottom: i64,
}

impl IntegerEdge {
    pub fn new(x0: i64, y0: i64, x1: i64, y1: i64) -> Self {
        Self {
            x0,
            y0,
            x1,
            y1,
            top: y0.min(y1),
            bottom: y0.max(y1),
        }
    }
}

impl From<Edge> for IntegerEdge {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rounding transient mask geometry to U"
    )]
    fn from(e: Edge) -> Self {
        let at = |v: f32| (f64::from(v) * 65536.0).round() as i64;
        Self::new(at(e.x0), at(e.y0), at(e.x1), at(e.y1))
    }
}
/// Interval count, vertical capacities and four coordinate bounds.
const HEAD: usize = 6;

/// A quantized edge: endpoints in 1/256 px relative to its tile's origin.
#[derive(Clone, Copy, Debug)]
struct QEdge {
    x0: i16,
    y0: i16,
    x1: i16,
    y1: i16,
}

impl QEdge {
    /// `[x0, y0, x1, y1]` packed little-endian into one word.
    fn word(self) -> Word {
        u64::from(self.x0.cast_unsigned())
            | u64::from(self.y0.cast_unsigned()) << 16
            | u64::from(self.x1.cast_unsigned()) << 32
            | u64::from(self.y1.cast_unsigned()) << 48
    }
}

/// The decoded edge `[x0, y0, x1, y1]` of the packed word `q` under a
/// tile origin given in units of 1/256 px.
#[expect(
    clippy::cast_possible_truncation,
    reason = "each word field is exactly one i16 lane; the one-rounding \
              conversion is the decode rule"
)]
#[inline]
fn decode_scalar([ox, oy]: [i32; 2], q: Word) -> IntegerEdge {
    let at = |origin: i32, q: i16| i64::from(origin + i32::from(q)) << 8;
    IntegerEdge::new(
        at(ox, q as i16),
        at(oy, (q >> 16) as i16),
        at(ox, (q >> 32) as i16),
        at(oy, (q >> 48) as i16),
    )
}

/// One immutable allocation containing bounds, normalized boundaries and their range index.
#[derive(Clone)]
pub struct EdgeTiles {
    words: Arc<[Word]>,
}

/// Realization scratch, released at the frame boundary.
#[derive(Default)]
pub struct EdgeScratch {
    decoded: Vec<IntegerEdge>,
    ends: Vec<i64>,
    sweep: super::topology::Sweep,
}

impl EdgeScratch {
    pub(crate) fn heap_bytes(&self) -> u64 {
        (self.decoded.capacity() * size_of::<IntegerEdge>()
            + self.ends.capacity() * size_of::<i64>()
            + self.sweep.heap_bytes()) as u64
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// The nearest unit of 1/256 px, ties rounded up: the tile origins are
/// multiples of 127 × 256 units, so the same frame point rounds to the
/// same unit from every tile, and the ±32512-unit range can never reach
/// the `i16` limits.
#[expect(
    clippy::cast_possible_truncation,
    clippy::suboptimal_flops,
    reason = "the float-to-int cast is the quantization rule; inputs stay in \
              ±32512. `v * 256 + 0.5` rounds once — the multiply is a power \
              of two and exact — so it equals `mul_add` without the libcall"
)]
fn quantize(v: f64) -> i16 {
    ifloor(v * UNIT + 0.5) as i16
}

/// `v.floor()` as an integer: truncation plus a one-step correction for
/// negative fractions — `cvttsd2si`, never a libcall.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "the correction compares against the truncated value; inputs are \
              tile and quantization magnitudes, far under 2^52"
)]
fn ifloor(v: f64) -> i64 {
    let t = v as i64;
    t - i64::from(v < t as f64)
}

/// The tile of the frame grid containing `(x, y)` — floor to a 127 px
/// boundary, so a point exactly on a boundary belongs to the tile
/// starting there.
fn tile_index(x: f64, y: f64) -> (i64, i64) {
    (ifloor(x / TILE), ifloor(y / TILE))
}

/// Emits `e`'s quantized pieces to `emit` as `(tile index, piece)`
/// triples: the part inside each crossed range boundary becomes its own
/// edge under the tile holding its start.
#[expect(
    clippy::cast_precision_loss,
    reason = "tile indices far under 2^52 become f64 origins exactly"
)]
fn split(e: &Edge, mut emit: impl FnMut(i16, i16, QEdge)) {
    let (mut x0, mut y0) = (f64::from(e.x0), f64::from(e.y0));
    let (x1, y1) = (f64::from(e.x1), f64::from(e.y1));
    loop {
        let (txi, tyi) = tile_index(x0, y0);
        let (tx, ty) = (txi as f64 * TILE, tyi as f64 * TILE);
        // The first range boundary the open end crosses. The far side
        // (+127) is a grid line, so snapping the exit coordinate lands
        // the next piece's start inside the neighbouring tile exactly —
        // interpolation rounding cannot put it back in this one.
        let (rx, ry) = (x0 - tx, y0 - ty);
        let (ex, ey) = (x1 - tx, y1 - ty);
        let (mut t, mut snap_x, mut snap_y) = (f64::INFINITY, None, None);
        if ex > TILE {
            t = (TILE - rx) / (ex - rx);
            snap_x = Some(tx + TILE);
        } else if ex < -TILE {
            t = (-TILE - rx) / (ex - rx);
            snap_x = Some(tx - TILE);
        }
        if ey > TILE {
            let s = (TILE - ry) / (ey - ry);
            if s < t {
                t = s;
                snap_x = None;
                snap_y = Some(ty + TILE);
            }
        } else if ey < -TILE {
            let s = (-TILE - ry) / (ey - ry);
            if s < t {
                t = s;
                snap_x = None;
                snap_y = Some(ty - TILE);
            }
        }
        let inside = t >= 1.0;
        let (px, py) = if inside {
            (x1, y1)
        } else {
            (
                snap_x.unwrap_or_else(|| t.mul_add(x1 - x0, x0)),
                snap_y.unwrap_or_else(|| t.mul_add(y1 - y0, y0)),
            )
        };
        let q = QEdge {
            x0: quantize(x0 - tx),
            y0: quantize(y0 - ty),
            x1: quantize(px - tx),
            y1: quantize(py - ty),
        };
        // Quantization can flatten a piece; horizontal edges carry no
        // area and drop out as in `flatten_edges`.
        if q.y0 != q.y1 {
            emit(
                i16::try_from(txi).expect("frame-space tile index exceeds ±32767"),
                i16::try_from(tyi).expect("frame-space tile index exceeds ±32767"),
                q,
            );
        }
        if inside {
            break;
        }
        x0 = px;
        y0 = py;
    }
}

impl EdgeTiles {
    pub fn compile(scratch: &mut EdgeScratch, edges: &[Edge], rule: cherenkov::FillRule) -> Self {
        let rule = (!cherenkov::lowering::proves_disjoint(edges)).then_some(rule);
        Self::build(scratch, edges.iter().copied(), rule)
    }

    #[cfg(test)]
    pub fn new(scratch: &mut EdgeScratch, edges: impl Iterator<Item = Edge>) -> Self {
        Self::build(scratch, edges, None)
    }

    fn build(
        scratch: &mut EdgeScratch,
        edges: impl Iterator<Item = Edge>,
        rule: Option<cherenkov::FillRule>,
    ) -> Self {
        scratch.decoded.clear();
        for e in edges {
            split(&e, |tx, ty, q| {
                scratch.decoded.push(decode_scalar(
                    [i32::from(tx) * TILE_UNITS, i32::from(ty) * TILE_UNITS],
                    q.word(),
                ));
            });
        }
        let mut bounds = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
        for e in &scratch.decoded {
            bounds[0] = bounds[0].min(e.x0.min(e.x1));
            bounds[1] = bounds[1].min(e.y0.min(e.y1));
            bounds[2] = bounds[2].max(e.x0.max(e.x1));
            bounds[3] = bounds[3].max(e.y0.max(e.y1));
        }
        if let Some(rule) = rule {
            scratch.sweep.compile(&scratch.decoded, rule);
            Self::finish(
                scratch.sweep.output.iter().map(|visible| {
                    let mut e = scratch.decoded[visible.source];
                    if (e.y0 < e.y1) != (visible.sign > 0) {
                        std::mem::swap(&mut e.x0, &mut e.x1);
                        std::mem::swap(&mut e.y0, &mut e.y1);
                    }
                    e.top = visible.top;
                    e.bottom = visible.bottom;
                    Boundary::new(e)
                }),
                &mut scratch.ends,
                bounds,
            )
        } else {
            Self::finish(
                scratch.decoded.iter().copied().map(Boundary::new),
                &mut scratch.ends,
                bounds,
            )
        }
    }

    /// Transient masks already carry exact placement-grid edges.
    pub fn from_boundaries(edges: &[IntegerEdge]) -> Self {
        let prepared = edges
            .iter()
            .copied()
            .filter(|e| e.top < e.bottom)
            .map(Boundary::new);
        // Supporting endpoints conservatively bound every visible interval.
        // Transient intersections are draws as well as masks, so their
        // retained bounds must remain valid for culling and band selection.
        let bounds = edges.iter().filter(|e| e.top < e.bottom).fold(
            [i64::MAX, i64::MAX, i64::MIN, i64::MIN],
            |[x0, y0, x1, y1], e| {
                [
                    x0.min(e.x0.min(e.x1)),
                    y0.min(e.top),
                    x1.max(e.x0.max(e.x1)),
                    y1.max(e.bottom),
                ]
            },
        );
        Self::finish(prepared, &mut Vec::new(), bounds)
    }

    fn finish(
        mut prepared: impl Iterator<Item = Boundary> + Clone,
        ends: &mut Vec<i64>,
        bounds: [i64; 4],
    ) -> Self {
        let count = prepared.clone().count();
        let nodes = intervals::leaves(count) * 2;
        let mut words = Arc::<[Word]>::new_uninit_slice(HEAD + count * 4 + nodes);
        let out = Arc::get_mut(&mut words).expect("fresh allocation");
        let (head, data) = out.split_at_mut(HEAD);
        let (payload, tree) = data.split_at_mut(count * 4);
        head[0].write(u64::try_from(count).expect("boundary count fits u64"));
        for (slot, value) in head[2..].iter_mut().zip(bounds) {
            slot.write(value.cast_unsigned());
        }
        // Initialize through checked word slices before viewing the payload as
        // boundaries; iterator length is not relied on for memory safety.
        for slot in payload.as_chunks_mut::<4>().0 {
            let boundary = prepared.next().expect("exact boundary count");
            for (word, value) in slot.iter_mut().zip(boundary.words()) {
                word.write(value);
            }
        }
        // SAFETY: Boundary is repr(C), consists of four eight-byte fields,
        // and has the same alignment as Word. The loop above initializes the
        // entire payload. The tree and payload are disjoint.
        unsafe {
            let prepared =
                std::slice::from_raw_parts_mut(payload.as_mut_ptr().cast::<Boundary>(), count);
            prepared.sort_unstable_by_key(|b| b.top());
            let (band, window) = intervals::vertical_capacity(prepared, ends);
            head[1].write(
                u64::from(u32::try_from(band).expect("band count fits u32"))
                    | u64::from(u32::try_from(window).expect("interval count fits u32")) << 32,
            );
            let tree = std::slice::from_raw_parts_mut(tree.as_mut_ptr().cast::<Word>(), nodes);
            intervals::tree(tree, prepared);
        }
        Self {
            // SAFETY: header, boundary payload and every tree node are initialized.
            words: unsafe { words.assume_init() },
        }
    }

    pub fn boundaries(&self) -> &[Boundary] {
        // SAFETY: finish initializes this aligned Boundary payload in place;
        // it remains immutable for the lifetime of the Arc.
        unsafe { std::slice::from_raw_parts(self.words.as_ptr().add(HEAD).cast(), self.len()) }
    }

    pub fn visit(&self, top: i64, bottom: i64, emit: impl FnMut(Boundary)) {
        intervals::visit(
            self.boundaries(),
            &self.words[HEAD + self.len() * 4..],
            top,
            bottom,
            emit,
        );
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "two u32 counts packed in one word"
    )]
    pub fn vertical_capacity(&self, windowed: bool) -> usize {
        if windowed {
            (self.words[1] >> 32) as usize
        } else {
            self.words[1] as u32 as usize
        }
    }

    pub fn bounds(&self) -> [i64; 4] {
        self.words[2..HEAD]
            .try_into()
            .map(|v: [u64; 4]| v.map(u64::cast_signed))
            .unwrap()
    }

    #[expect(
        clippy::cast_possible_truncation,
        reason = "count was written from usize on this machine"
    )]
    pub fn len(&self) -> usize {
        self.words[0] as usize
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    #[cfg(test)]
    pub(super) fn identity(&self) -> usize {
        Arc::as_ptr(&self.words).cast::<u64>() as usize
    }

    pub(super) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.words)
    }
    pub fn heap_bytes(&self) -> u64 {
        (self.words.len() * size_of::<Word>() + 2 * size_of::<usize>()) as u64
    }

    #[cfg(test)]
    #[expect(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        reason = "test conversion to pixels"
    )]
    pub fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        self.boundaries().iter().map(|b| {
            let (x0, y0, x1, y1) = (
                b.x / 65536.0,
                b.top() as f64 / 65536.0,
                ((b.bottom - b.top()) as f64).mul_add(b.slope, b.x) / 65536.0,
                b.bottom as f64 / 65536.0,
            );
            if b.sign() > 0 {
                Edge {
                    x0: x0 as f32,
                    y0: y0 as f32,
                    x1: x1 as f32,
                    y1: y1 as f32,
                }
            } else {
                Edge {
                    x0: x1 as f32,
                    y0: y1 as f32,
                    x1: x0 as f32,
                    y1: y0 as f32,
                }
            }
        })
    }
}

impl std::fmt::Debug for EdgeTiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EdgeTiles")
            .field("boundaries", &self.len())
            .field("bounds", &self.bounds())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transient_boundary_geometry_retains_device_bounds() {
        let raw = [
            IntegerEdge::new(5 * ONE, 0, 5 * ONE, 10 * ONE),
            IntegerEdge::new(10 * ONE, 10 * ONE, 10 * ONE, 0),
        ];
        let edges = EdgeTiles::from_boundaries(&raw);
        assert_eq!(edges.bounds(), [5 * ONE, 0, 10 * ONE, 10 * ONE]);
        assert!(!edges.is_empty());
    }

    #[test]
    fn interval_queries_match_exact_translated_ranges() {
        let raw: Vec<_> = (0..257_i64)
            .map(|i| {
                let top = (i * 977) % (80 * ONE) - 32 * ONE;
                let bottom = top + 1 + (i * 7919) % (48 * ONE);
                IntegerEdge::new(i * ONE, top, i * ONE, bottom)
            })
            .collect();
        let edges = EdgeTiles::from_boundaries(&raw);
        for offset in [-33 * ONE, -1, 0, 1, 17 * ONE + 19] {
            for height in [1, 3, 16, 37] {
                for row in -40..96 {
                    let top = row * ONE - offset;
                    let bottom = top + height * ONE;
                    let expected: Vec<_> = edges
                        .boundaries()
                        .iter()
                        .filter(|b| b.top() < bottom && b.bottom > top)
                        .map(|b| b.x.to_bits())
                        .collect();
                    let mut actual = Vec::new();
                    edges.visit(top, bottom, |b| actual.push(b.x.to_bits()));
                    assert_eq!(actual, expected);
                    assert!(actual.len() <= edges.vertical_capacity(height > 16));
                }
            }
        }
        let empty = EdgeTiles::from_boundaries(&[]);
        empty.visit(-ONE, ONE, |_| panic!("empty geometry has no intervals"));
    }

    fn edge(x0: f32, y0: f32, x1: f32, y1: f32) -> Edge {
        Edge { x0, y0, x1, y1 }
    }

    fn tiled(input: &[Edge]) -> EdgeTiles {
        EdgeTiles::new(&mut EdgeScratch::default(), input.iter().copied())
    }

    #[test]
    fn compiled_intervals_keep_the_original_line_at_crossings() {
        let input = [
            edge(0.25, 0.125, 6.75, 7.875),
            edge(6.75, 7.875, 0.5, 7.0),
            edge(0.5, 7.0, 7.0, 0.25),
            edge(7.0, 0.25, 0.25, 0.125),
        ];
        let tiles = EdgeTiles::compile(
            &mut EdgeScratch::default(),
            &input,
            cherenkov::FillRule::NonZero,
        );
        let expected =
            crate::render::raster::coverage_mask(&input, cherenkov::FillRule::NonZero, 8, 8);
        for band in [3, 8] {
            let mut actual = vec![0.0f32; 64];
            let mut compiler = crate::render::coverage::Compiler::default();
            for top in (0..8).step_by(band) {
                let end = (top + band).min(8);
                compiler.prepare(&tiles, Fixed::default(), top..end, 8);
                compiler.rows(end - top, |y, range, run| {
                    for (i, x) in range.enumerate() {
                        actual[(top + y) * 8 + x] = run.at(i);
                    }
                });
            }
            assert!(
                actual
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| (a - b).abs() <= 1e-6)
            );
        }
    }

    /// The largest coordinate difference between two edges.
    fn distance(a: &Edge, b: &Edge) -> f32 {
        (a.x0 - b.x0)
            .abs()
            .max((a.y0 - b.y0).abs())
            .max((a.x1 - b.x1).abs())
            .max((a.y1 - b.y1).abs())
    }

    /// Every decoded edge matches its input within half a unit —
    /// 1/512 px.
    #[test]
    fn round_trip_within_half_a_unit() {
        let input = [
            edge(1.0, 2.0, 37.31, 90.7),
            edge(200.0, 10.0, 5.5, 64.25),
            edge(64.0, 200.0, 64.25, 5.0),
            edge(0.125, 300.0, 99.75, 129.5),
        ];
        let tiles = tiled(&input);
        assert_eq!(tiles.len(), input.len());
        for q in tiles.iter() {
            let nearest = input
                .iter()
                .map(|e| distance(e, &q))
                .fold(f32::INFINITY, f32::min);
            assert!(nearest <= 1.0 / 512.0, "decoded {q:?} drifted {nearest}");
        }
    }

    /// An edge whose other endpoint leaves the tile's ±127 px range is
    /// split at the range boundary — a tile grid line — into one piece
    /// per tile the remainder enters.
    #[test]
    fn edge_longer_than_the_tile_range_splits() {
        let tiles = tiled(&[edge(10.0, 10.0, 500.0, 380.0)]);
        assert!(tiles.len() > 2, "expected several pieces: {tiles:?}");
        // The pieces tile the original segment: along its (monotone)
        // direction each piece ends where the next starts, within the
        // quantization step.
        let mut decoded: Vec<Edge> = tiles.iter().collect();
        decoded.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap());
        for pair in decoded.windows(2) {
            assert!((pair[0].x1 - pair[1].x0).abs() <= 1.0 / 127.0);
            assert!((pair[0].y1 - pair[1].y0).abs() <= 1.0 / 127.0);
        }
        assert!((decoded[0].x0 - 10.0).abs() <= 1.0 / 512.0);
        assert!((decoded.last().unwrap().x1 - 500.0).abs() <= 1.0 / 512.0);
    }

    /// An edge starting exactly on a tile boundary belongs to the tile
    /// starting there.
    #[test]
    fn start_on_a_tile_boundary() {
        let tiles = tiled(&[edge(127.0, 5.0, 140.0, 60.0)]);
        let e = tiles.iter().next().unwrap();
        assert_eq!(e.x0, 127.0);
        assert!((e.y0 - 5.0).abs() <= 1.0 / 512.0);
        // A corner start belongs to the tile at the corner.
        let tiles = tiled(&[edge(127.0, 127.0, 140.0, 150.0)]);
        let e = tiles.iter().next().unwrap();
        assert_eq!([e.x0, e.y0], [127.0, 127.0]);
    }

    /// Negative frame coordinates tile and decode like positive ones:
    /// floor to the 127 px grid, not truncation toward zero.
    #[test]
    fn negative_frame_coordinates() {
        let input = [
            edge(-50.0, -50.0, -20.0, -60.0),
            edge(-20.0, -10.0, -5.0, -60.0),
        ];
        let tiles = tiled(&input);
        // Both edges start in the (-127, -127) tile and stay in range.
        assert_eq!(tiles.len(), 2);
        for q in tiles.iter() {
            assert!(
                input.iter().any(|e| distance(e, &q) <= 1.0 / 512.0),
                "decoded {q:?}"
            );
        }
    }

    /// A placement offset moves the decoded edges, not the retained
    /// geometry: the same allocation serves every translation.
    #[test]
    fn translation_reuses_the_same_geometry() {
        let geometry = tiled(&[edge(1.0, 2.0, 40.0, 60.0), edge(40.0, 60.0, 90.0, 12.5)]);
        let moved = geometry.clone();
        assert_eq!(geometry.identity(), moved.identity());
        let at = |offset: [f32; 2]| -> Vec<(f32, f32, f32, f32)> {
            moved
                .iter()
                .map(|e| {
                    (
                        e.x0 + offset[0],
                        e.y0 + offset[1],
                        e.x1 + offset[0],
                        e.y1 + offset[1],
                    )
                })
                .collect()
        };
        let (a, b) = (at([10.0, 20.0]), at([13.25, -40.5]));
        for (a, b) in a.iter().zip(&b) {
            assert!((a.0 + 3.25 - b.0).abs() <= 1.0 / 256.0);
            assert!((a.1 - 60.5 - b.1).abs() <= 1.0 / 256.0);
        }
    }

    /// The stored bounds are the decoded edges' extrema — what the
    /// placement's `bbox_of` reduced per edge.
    #[test]
    fn bounds_match_the_decoded_extrema() {
        let tiles = tiled(&[
            edge(1.0, 2.0, 37.31, 90.7),
            edge(200.0, 10.0, 5.5, 64.25),
            edge(-64.0, 200.0, -64.25, 5.0),
        ]);
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for e in tiles.iter() {
            x0 = x0.min(e.x0.min(e.x1));
            y0 = y0.min(e.y0.min(e.y1));
            x1 = x1.max(e.x0.max(e.x1));
            y1 = y1.max(e.y0.max(e.y1));
        }
        assert_eq!(
            tiles.bounds(),
            [x0, y0, x1, y1].map(|v| IntegerEdge::from(edge(v, v, v, v)).x0)
        );
    }

    /// Many disjoint closed rectangles whose sides each cross two tile
    /// rows downward and back up, on fractional coordinates including
    /// exact half units: the decoded form must keep coverage within
    /// 1/128 per pixel — the encode loses no winding along any row.
    #[expect(
        clippy::cast_precision_loss,
        reason = "sub-pixel test offsets; the range is tiny"
    )]
    #[test]
    fn rectangles_keep_coverage_across_tile_rows() {
        let mut edges = Vec::new();
        for i in 0..120 {
            // 3 px wide, y 50..300, fractional offsets with exact half
            // units.
            let x = (i as f32).mul_add(5.0, 2.0) + (i % 7) as f32 / 512.0;
            let y0 = 50.0 + (i % 5) as f32 / 512.0;
            let y1 = 300.0 + (i % 3) as f32 / 512.0;
            edges.push(edge(x, y0, x + 3.0, y0));
            edges.push(edge(x + 3.0, y0, x + 3.0, y1));
            edges.push(edge(x + 3.0, y1, x, y1));
            edges.push(edge(x, y1, x, y0));
        }
        let tiles = tiled(&edges);
        let decoded: Vec<Edge> = tiles.iter().collect();
        let (w, h) = (640usize, 360usize);
        let a = crate::render::raster::coverage_mask(&edges, cherenkov::FillRule::NonZero, w, h);
        let b = crate::render::raster::coverage_mask(&decoded, cherenkov::FillRule::NonZero, w, h);
        let worst = a
            .iter()
            .zip(&b)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(worst <= 1.0 / 128.0, "worst coverage delta {worst}");
    }
}
