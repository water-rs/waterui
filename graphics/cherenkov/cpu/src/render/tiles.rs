//! Retained visible boundaries: original frame-space lines quantized to
//! 1/256 px and grouped by fixed 127 × 127 px tiles, with visible intervals
//! clipped on the 2^-16 px grid without rounding their x intersections.
//!
//! An [`Item::Draw`](crate::render::lower::Item::Draw) retains its
//! flattened segments for the life of its realization, and a placement
//! that differs only by a translation reuses them through the draw's
//! `offset`. The tiles are therefore anchored in frame space — a grid of
//! 127 px squares whose origins are integer multiples of 127 — never in
//! the band grid the placement lands on.
//!
//! Each edge belongs to the tile containing its first endpoint and stores
//! its four coordinates as `i16` units of 1/256 px relative to the tile's
//! origin: eight bytes an edge, half the `f32` form. The range reaches
//! 127 px past the origin in either direction, so a start inside the tile
//! always encodes; an edge whose other endpoint leaves the range is split
//! at the range boundary — a grid line, so the piece that follows starts
//! inside a tile again — into as many pieces as the crossing needs. 127,
//! not 128, keeps every encodable point strictly inside the `i16` range:
//! a piece ending on a boundary encodes as ±32512, never saturating, so
//! the piece that follows decodes to the same point.
//!
//! A draw's retained form is a single `Arc<[Word]>` allocation: a header
//! word (tile and edge counts), two words of decoded-coordinate bounds,
//! one word per tile (`tx: i16 | ty: i16 | end: u32`), then one word per
//! edge. A sparse tail stores `(edge index, top/bottom distances)` in two
//! words for each clipped interval; complete edges need no extra storage.
//! Hidden edges are discarded, not retained alongside the boundary stream.
//! Tile indices are `i16`: frame coordinates beyond ±4,161,409 px
//! fail the encoding — the forms a draw retains come from a `Frame` whose
//! contents sit next to its origin.
//!
//! Decoding produces exact integer 1/256 px coordinates. Placement widens
//! them to 2^-16 px units before adding the draw's fixed offset.

use std::mem::MaybeUninit;
use std::sync::Arc;

use fearless_simd::{Simd, i32x4};

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
/// Header and bounds words before the tile words.
const HEAD: usize = 3;

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

/// Decodes four signed coordinates in 1/256 px units. The caller widens
/// before converting to placement units and adding its translation.
#[expect(
    clippy::inline_always,
    clippy::cast_possible_truncation,
    reason = "compiled under the dispatched CPU level like its callers; \
              each word field is exactly one packed i16 lane"
)]
#[inline(always)]
pub fn decode<S: Simd>(simd: S, origin: i32x4<S>, q: Word) -> i32x4<S> {
    let v = simd.load_array_i16x8([
        q as i16,
        (q >> 16) as i16,
        (q >> 32) as i16,
        (q >> 48) as i16,
        0,
        0,
        0,
        0,
    ]);
    // Pair each 16-bit lane with itself, then read it as an `i32`: the
    // arithmetic shift right sign-extends the lane — `vpunpcklwd` +
    // `vpsrad` — giving `[x0, y0, x1, y1]` as `i32`.
    let q = simd.cvt_from_bytes_i32x4(simd.reinterpret_u8_i16x8(simd.zip_low_i16x8(v, v)));
    let q = simd.shr_i32x4(q, 16);
    simd.add_i32x4(q, origin)
}

/// A draw's flattened edges in retained, tile-local fixed point, as one
/// `Arc` allocation. Clones share it, like an `Arc`.
#[derive(Clone)]
pub struct EdgeTiles {
    /// `[header, bounds × 2, tiles, edges]`: the header's low half is the
    /// tile count and its high half the edge count; each tile word is
    /// `tx: i16 | ty: i16 | end: u32`, `end` the edge index where the
    /// next tile begins.
    words: Arc<[Word]>,
}

/// Scratch an [`EdgeTiles::compile`] builds into, reused within a lowering
/// pass and released at the frame boundary.
#[derive(Default)]
pub struct EdgeScratch {
    /// `(packed tile index, edge word)` pairs in emission order.
    pieces: Vec<(u32, Word)>,
    /// Per dense-tile counts, then that tile's write position in the
    /// retained layout.
    counts: Vec<u32>,
    decoded: Vec<IntegerEdge>,
    resolved: Vec<(u32, Word)>,
    limits: Vec<(u32, u32)>,
    cuts: Vec<(u32, u64)>,
    sweep: super::topology::Sweep,
}

impl EdgeScratch {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "packed tile key contains two i16 lanes"
    )]
    fn resolve(&mut self, rule: cherenkov::FillRule) {
        self.decoded.clear();
        self.decoded.extend(self.pieces.iter().map(|&(key, word)| {
            decode_scalar(
                [
                    i32::from((key >> 16) as i16) * TILE_UNITS,
                    i32::from(key as i16) * TILE_UNITS,
                ],
                word,
            )
        }));
        self.sweep.compile(&self.decoded, rule);
        self.resolved.clear();
        for visible in &self.sweep.output {
            let (key, mut word) = self.pieces[visible.source];
            let original = self.decoded[visible.source];
            if (original.y0 < original.y1) != (visible.sign > 0) {
                word = word.rotate_left(32);
            }
            self.resolved.push((key, word));
            self.limits.push((
                u32::try_from(visible.top - original.top)
                    .expect("visible interval lies on its source"),
                u32::try_from(original.bottom - visible.bottom)
                    .expect("visible interval lies on its source"),
            ));
        }
        std::mem::swap(&mut self.pieces, &mut self.resolved);
    }

    /// Retained high-water capacity: a renderer buffer, not per draw.
    pub(crate) fn heap_bytes(&self) -> u64 {
        (self.pieces.capacity() * size_of::<(u32, Word)>()
            + self.counts.capacity() * size_of::<u32>()
            + self.decoded.capacity() * size_of::<IntegerEdge>()
            + self.resolved.capacity() * size_of::<(u32, Word)>()
            + self.limits.capacity() * size_of::<(u32, u32)>()
            + self.cuts.capacity() * size_of::<(u32, u64)>()
            + self.sweep.heap_bytes()) as u64
    }

    /// Releases the storage at a frame boundary: reuse is within one
    /// lowering pass, and a large draw's buffers are not retained between
    /// frames.
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

/// The order a `(origin_x_bits, origin_y_bits)` sort produced, mapped to
/// `0..hi - lo + 1` dense keys: non-negative tile indices ascending, then
/// negatives in ascending magnitude — `f32` bits order the negative half
/// from nearest to farthest.
#[expect(
    clippy::cast_sign_loss,
    reason = "the rank is an index into the dense grid by construction"
)]
const fn rank(t: i32, lo: i32, hi: i32) -> usize {
    let r = if lo >= 0 {
        t - lo
    } else if hi < 0 {
        -1 - t
    } else if t >= 0 {
        t
    } else {
        hi - t
    };
    r as usize
}

/// The tile index of a dense `rank` — the inverse of [`rank`]. `rank`
/// stays under 2^16 per axis, so the `usize → i32` conversion is exact.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "ranks are dense indices under the 2^16-wide tile grid"
)]
const fn unrank(r: usize, lo: i32, hi: i32) -> i32 {
    let r = r as i32;
    if lo >= 0 {
        lo + r
    } else if hi < 0 {
        -1 - r
    } else if r <= hi {
        r
    } else {
        hi - r
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

    /// Quantizes `edges` — finite frame-space `f32` segments — into
    /// tile-local fixed point, in one pass over `edges` and three over
    /// `scratch`, whose storage persists across calls.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::similar_names,
        reason = "tile indices are checked i16; counts and positions are \
                  bounded by the edge count, itself checked u32"
    )]
    fn build(
        scratch: &mut EdgeScratch,
        edges: impl Iterator<Item = Edge>,
        rule: Option<cherenkov::FillRule>,
    ) -> Self {
        let pieces = &mut scratch.pieces;
        pieces.clear();
        // Tile-grid extent, tracked as pieces emit; `bounds` is the
        // decoded extrema `bbox_of` reduced over.
        let (mut tx_lo, mut tx_hi, mut ty_lo, mut ty_hi) = (i32::MAX, i32::MIN, i32::MAX, i32::MIN);
        let mut bounds = [i32::MAX, i32::MAX, i32::MIN, i32::MIN];
        for e in edges {
            split(&e, |tx, ty, q| {
                tx_lo = tx_lo.min(i32::from(tx));
                tx_hi = tx_hi.max(i32::from(tx));
                ty_lo = ty_lo.min(i32::from(ty));
                ty_hi = ty_hi.max(i32::from(ty));
                let origin = [i32::from(tx) * TILE_UNITS, i32::from(ty) * TILE_UNITS];
                let w = q.word();
                let d = decode_scalar(origin, w);
                bounds[0] = bounds[0].min((d.x0.min(d.x1) >> 8) as i32);
                bounds[1] = bounds[1].min((d.y0.min(d.y1) >> 8) as i32);
                bounds[2] = bounds[2].max((d.x0.max(d.x1) >> 8) as i32);
                bounds[3] = bounds[3].max((d.y0.max(d.y1) >> 8) as i32);
                pieces.push((
                    u32::from(tx.cast_unsigned()) << 16 | u32::from(ty.cast_unsigned()),
                    w,
                ));
            });
        }
        scratch.limits.clear();
        if let Some(rule) = rule {
            scratch.resolve(rule);
        }
        let pieces = &scratch.pieces;
        let n = u32::try_from(pieces.len()).expect("a draw's edges fit u32");
        let write_bounds = |out: &mut [MaybeUninit<Word>]| {
            out[1].write(
                u64::from(bounds[0].cast_unsigned()) | u64::from(bounds[1].cast_unsigned()) << 32,
            );
            out[2].write(
                u64::from(bounds[2].cast_unsigned()) | u64::from(bounds[3].cast_unsigned()) << 32,
            );
        };
        if n == 0 {
            let mut words = Arc::<[Word]>::new_uninit_slice(HEAD);
            let out = Arc::get_mut(&mut words).expect("a fresh Arc is unique");
            out[0].write(0);
            write_bounds(out);
            // SAFETY: the three header/bounds words were written above.
            let words = unsafe { words.assume_init() };
            return Self { words };
        }
        // Dense keys over the draw's tile grid, ordering tiles as the
        // former `(ox_bits, oy_bits)` sort did so each piece keeps its
        // edge index.
        let (xe, ye) = ((tx_hi - tx_lo + 1) as usize, (ty_hi - ty_lo + 1) as usize);
        let dense = |(tx, ty): (i32, i32)| rank(tx, tx_lo, tx_hi) * ye + rank(ty, ty_lo, ty_hi);
        let key = |k: u32| (i32::from((k >> 16) as i16), i32::from(k as i16));
        let counts = &mut scratch.counts;
        counts.clear();
        counts.resize(xe * ye, 0);
        for &(k, _) in pieces {
            counts[dense(key(k))] += 1;
        }
        let ntiles = counts.iter().filter(|&&c| c != 0).count();
        // The retained storage is written in place: tile words in the
        // prefix pass, edge words in the scatter.
        let ncuts = scratch
            .limits
            .iter()
            .filter(|&&(a, b)| a != 0 || b != 0)
            .count();
        let mut words = Arc::<[Word]>::new_uninit_slice(HEAD + ntiles + n as usize + ncuts * 2);
        let out = Arc::get_mut(&mut words).expect("a fresh Arc is unique");
        // Prefix pass: tile words and each tile's write position.
        let (mut t, mut running) = (0usize, 0u32);
        for (d, c) in counts.iter_mut().enumerate() {
            if *c == 0 {
                continue;
            }
            let (tx, ty) = (unrank(d / ye, tx_lo, tx_hi), unrank(d % ye, ty_lo, ty_hi));
            running += *c;
            out[HEAD + t].write(
                u64::from(tx as u16) | u64::from(ty as u16) << 16 | u64::from(running) << 32,
            );
            *c = (HEAD + ntiles) as u32 + running - *c;
            t += 1;
        }
        // Scatter: a stable counting sort — pieces in emission order keep
        // each tile's order.
        scratch.cuts.clear();
        for (i, &(k, w)) in pieces.iter().enumerate() {
            let d = dense(key(k));
            out[counts[d] as usize].write(w);
            if let Some(&(a, b)) = scratch.limits.get(i)
                && (a != 0 || b != 0)
            {
                scratch.cuts.push((
                    counts[d] - (HEAD + ntiles) as u32,
                    u64::from(a) | u64::from(b) << 32,
                ));
            }
            counts[d] += 1;
        }
        scratch.cuts.sort_unstable_by_key(|&(i, _)| i);
        for (slot, &(i, limits)) in scratch.cuts.iter().enumerate() {
            out[HEAD + ntiles + n as usize + slot * 2].write(u64::from(i));
            out[HEAD + ntiles + n as usize + slot * 2 + 1].write(limits);
        }
        out[0].write(u64::from(ntiles as u32) | u64::from(n) << 32);
        write_bounds(out);
        // SAFETY: the tile write positions are the counting sort's prefix,
        // so the scatter wrote every slot of `HEAD + ntiles .. len`
        // exactly once, and the header, bounds and tile words were
        // written explicitly.
        let words = unsafe { words.assume_init() };
        Self { words }
    }

    /// `(tiles, edges)` counts.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "each header field is exactly one packed u32"
    )]
    fn header(&self) -> (usize, usize) {
        (
            self.words[0] as u32 as usize,
            (self.words[0] >> 32) as usize,
        )
    }

    /// The tile words, ordered by their grid position as the emission's
    /// `(ox_bits, oy_bits)` sort produced.
    fn tiles(&self) -> &[Word] {
        &self.words[HEAD..HEAD + self.header().0]
    }

    /// The packed edge words, grouped by tile.
    fn edges(&self) -> &[Word] {
        let (tiles, edges) = self.header();
        &self.words[HEAD + tiles..HEAD + tiles + edges]
    }

    fn cuts(&self) -> &[Word] {
        let (tiles, edges) = self.header();
        &self.words[HEAD + tiles + edges..]
    }

    /// `[x_min, y_min, x_max, y_max]` over the decoded coordinates,
    /// recorded at build time — the extrema `bbox_of` reduced per edge.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "each bounds field is exactly one packed i32 lane"
    )]
    pub fn bounds(&self) -> [i64; 4] {
        [
            i64::from((self.words[1] as u32).cast_signed()) << 8,
            i64::from(((self.words[1] >> 32) as u32).cast_signed()) << 8,
            i64::from((self.words[2] as u32).cast_signed()) << 8,
            i64::from(((self.words[2] >> 32) as u32).cast_signed()) << 8,
        ]
    }

    /// The edge count, split pieces included.
    pub fn len(&self) -> usize {
        self.header().1
    }

    /// Whether no edge survived quantization.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A forward reader over the tiles and edges.
    pub fn cursor(&self) -> TileCursor<'_> {
        TileCursor {
            tiles: self.tiles(),
            edges: self.edges(),
            next: 0,
            end: 0,
            origin: [0; 2],
            pos: 0,
            cuts: self.cuts(),
            cut: 0,
        }
    }

    /// The decoded edges in storage order — tests and transient checks.
    #[cfg(test)]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "each word field is exactly one i16 lane"
    )]
    pub fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        let edges = self.edges();
        let mut start = 0usize;
        self.tiles().iter().flat_map(move |&t| {
            let end = (t >> 32) as usize;
            let range = start..end;
            start = end;
            let origin = [
                i32::from(t as i16) * TILE_UNITS,
                i32::from((t >> 16) as i16) * TILE_UNITS,
            ];
            edges[range]
                .iter()
                .map(move |&q| float_edge(decode_scalar(origin, q)))
        })
    }

    /// The retained allocation's identity: the `Arc`'s backing pointer.
    pub(super) fn identity(&self) -> usize {
        Arc::as_ptr(&self.words).cast::<u64>() as usize
    }

    /// The retained allocation's strong refcount.
    pub(super) fn strong_count(&self) -> usize {
        Arc::strong_count(&self.words)
    }

    /// The real heap size: one allocation of `words` plus the `Arc`'s
    /// refcount pair.
    pub fn heap_bytes(&self) -> u64 {
        (self.words.len() * size_of::<Word>() + 2 * size_of::<usize>()) as u64
    }
}

impl std::fmt::Debug for EdgeTiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (tiles, edges) = self.header();
        f.debug_struct("EdgeTiles")
            .field("tiles", &tiles)
            .field("edges", &edges)
            .field("bounds", &self.bounds())
            .finish()
    }
}

/// A forward reader over an [`EdgeTiles`]: `seek` places the read index
/// by binary search over the tile ends, then sequential reads advance
/// the current tile only when it is left behind — amortized O(1) per
/// edge, O(tiles) per scan.
pub struct TileCursor<'a> {
    tiles: &'a [Word],
    edges: &'a [Word],
    /// The next tile word to load once the read index leaves `end`.
    next: usize,
    /// The current tile's past-last edge index — `0` before any load, so
    /// the first read always loads a tile.
    end: u64,
    /// The current tile's origin in units of 1/256 px.
    origin: [i32; 2],
    /// The last index read: reads never go back.
    pos: usize,
    cuts: &'a [Word],
    cut: usize,
}

impl<'a> TileCursor<'a> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "packed endpoint distances are u32 lanes"
    )]
    fn limit(&mut self, i: usize, mut edge: IntegerEdge) -> IntegerEdge {
        while self.cut < self.cuts.len() && self.cuts[self.cut] < i as u64 {
            self.cut += 2;
        }
        if self.cut < self.cuts.len() && self.cuts[self.cut] == i as u64 {
            let limits = self.cuts[self.cut + 1];
            edge.top += i64::from(limits as u32);
            edge.bottom -= i64::from((limits >> 32) as u32);
        }
        edge
    }
    /// Advances the current tile to the one holding `i` — a while-loop,
    /// not a search: every step is a tile the scan leaves behind.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "each word field is exactly one i16 lane"
    )]
    #[inline]
    fn position(&mut self, i: usize) {
        debug_assert!(i >= self.pos, "an edge cursor reads forward");
        self.pos = i;
        while i as u64 >= self.end {
            let w = self.tiles[self.next];
            self.origin = [
                i32::from(w as i16) * TILE_UNITS,
                i32::from((w >> 16) as i16) * TILE_UNITS,
            ];
            self.end = w >> 32;
            self.next += 1;
        }
    }

    /// Advances the current tile to the one holding `i` and returns its
    /// origin in units of 1/256 px, its past-last edge index, and its
    /// edge words starting at `i`.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the tile word's end field is exactly one packed u32"
    )]
    #[inline]
    pub(crate) fn run(&mut self, i: usize) -> ([i32; 2], usize, &'a [Word]) {
        self.position(i);
        (
            self.origin,
            self.end as usize,
            &self.edges[i..self.end as usize],
        )
    }
}

/// An edge list the band bins index: retained tile-local geometry
/// ([`EdgeTiles`]) or transient `f32` edges — clip coverage masks,
/// silhouette captures — that are never retained (`[Edge]`).
pub trait EdgeSource: Sync {
    /// The forward reader this source produces.
    type Cursor<'a>: EdgeCursor
    where
        Self: 'a;
    /// Visits a sorted band selection under the renderer's SIMD token.
    fn selected<S: Simd>(&self, _simd: S, selection: &[u32], mut emit: impl FnMut(IntegerEdge)) {
        let mut cursor = self.cursor();
        if let Some(&first) = selection.first() {
            cursor.seek(first as usize);
        }
        for &i in selection {
            emit(cursor.at(i as usize));
        }
    }
    /// The list's edge count.
    fn len(&self) -> usize;
    /// A reader positioned at the start of the list.
    fn cursor(&self) -> Self::Cursor<'_>;
}

/// A forward reader over an edge list's decoded edges, produced by
/// [`EdgeSource::cursor`].
pub trait EdgeCursor {
    /// Places the reader at edge `i` — the first index a scan reads.
    fn seek(&mut self, i: usize);
    /// Decoded edge `i`: a reader is called with ascending indices only.
    fn at(&mut self, i: usize) -> IntegerEdge;
}

impl EdgeSource for EdgeTiles {
    type Cursor<'a> = TileCursor<'a>;

    fn selected<S: Simd>(&self, simd: S, selection: &[u32], mut emit: impl FnMut(IntegerEdge)) {
        let mut cursor = self.cursor();
        if let Some(&first) = selection.first() {
            cursor.seek(first as usize);
        }
        let mut i = 0;
        while i < selection.len() {
            let (origin, end, words) = cursor.run(selection[i] as usize);
            let base = selection[i] as usize;
            let origin = simd.load_array_i32x4([origin[0], origin[1], origin[0], origin[1]]);
            while i < selection.len() && (selection[i] as usize) < end {
                let [x0, y0, x1, y1] = simd
                    .as_array_i32x4(decode(simd, origin, words[selection[i] as usize - base]))
                    .map(|v| i64::from(v) << 8);
                emit(cursor.limit(selection[i] as usize, IntegerEdge::new(x0, y0, x1, y1)));
                i += 1;
            }
        }
    }

    fn len(&self) -> usize {
        Self::len(self)
    }

    fn cursor(&self) -> TileCursor<'_> {
        Self::cursor(self)
    }
}

impl EdgeCursor for TileCursor<'_> {
    fn seek(&mut self, i: usize) {
        self.next = self.tiles.partition_point(|&w| w >> 32 <= i as u64);
        self.end = 0;
        self.pos = i;
        let (mut lo, mut hi) = (0, self.cuts.len() / 2);
        while lo < hi {
            let middle = lo + (hi - lo) / 2;
            if self.cuts[middle * 2] < i as u64 {
                lo = middle + 1;
            } else {
                hi = middle;
            }
        }
        self.cut = lo * 2;
    }

    fn at(&mut self, i: usize) -> IntegerEdge {
        self.position(i);
        self.limit(i, decode_scalar(self.origin, self.edges[i]))
    }
}

impl EdgeSource for [Edge] {
    type Cursor<'a> = &'a [Edge];

    fn len(&self) -> usize {
        <[Edge]>::len(self)
    }

    fn cursor(&self) -> &[Edge] {
        self
    }
}

impl EdgeCursor for &[Edge] {
    fn seek(&mut self, _: usize) {}

    fn at(&mut self, i: usize) -> IntegerEdge {
        self[i].into()
    }
}

impl EdgeSource for [IntegerEdge] {
    type Cursor<'a> = &'a [IntegerEdge];
    fn len(&self) -> usize {
        <[IntegerEdge]>::len(self)
    }
    fn cursor(&self) -> &[IntegerEdge] {
        self
    }
}

impl EdgeCursor for &[IntegerEdge] {
    fn seek(&mut self, _: usize) {}
    fn at(&mut self, i: usize) -> IntegerEdge {
        self[i]
    }
}

#[cfg(test)]
#[expect(
    clippy::cast_precision_loss,
    reason = "test-only conversion back to pixels"
)]
fn float_edge(e: IntegerEdge) -> Edge {
    Edge {
        x0: e.x0 as f32 / 65536.0,
        y0: e.y0 as f32 / 65536.0,
        x1: e.x1 as f32 / 65536.0,
        y1: e.y1 as f32 / 65536.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Range;

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
        assert!(
            !tiles.cuts().is_empty(),
            "crossings produce clipped intervals"
        );
        let selection: Vec<_> = (0..u32::try_from(tiles.len()).unwrap()).collect();
        let expected =
            crate::render::raster::coverage_mask(&input, cherenkov::FillRule::NonZero, 8, 8);
        for band in [3, 8] {
            let mut actual = vec![0.0f32; 64];
            let mut compiler = crate::render::coverage::Compiler::default();
            for top in (0..8).step_by(band) {
                let end = (top + band).min(8);
                compiler.prepare(
                    fearless_simd::Fallback::new(),
                    &tiles,
                    &selection,
                    Fixed::default(),
                    top..end,
                    8,
                );
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

    /// Tile `t`: its decoded origin and its edge indices.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss,
        reason = "tile indices are i16 fields; px origins are exact in f32"
    )]
    fn tile_at(tiles: &EdgeTiles, t: usize) -> (f32, f32, Range<usize>) {
        let w = tiles.tiles()[t];
        let end = (w >> 32) as usize;
        let start = if t == 0 {
            0
        } else {
            (tiles.tiles()[t - 1] >> 32) as usize
        };
        (
            i32::from(w as i16) as f32 * TILE as f32,
            i32::from((w >> 16) as i16) as f32 * TILE as f32,
            start..end,
        )
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
        // Every piece's decoded start sits inside its own tile and every
        // decoded coordinate stays within ±127 px of the origin.
        for t in 0..tiles.tiles().len() {
            let (ox, oy, range) = tile_at(&tiles, t);
            for i in range {
                let mut cursor = tiles.cursor();
                let e = float_edge(EdgeCursor::at(&mut cursor, i));
                assert!((0.0..=127.0).contains(&(e.x0 - ox)));
                assert!((0.0..=127.0).contains(&(e.y0 - oy)));
                for rel in [e.x1 - ox, e.y1 - oy] {
                    assert!((-127.0..=127.0).contains(&rel), "{rel} out of range");
                }
            }
        }
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
        assert_eq!(tiles.header().0, 1);
        let (ox, oy, ..) = tile_at(&tiles, 0);
        assert_eq!([ox, oy], [127.0, 0.0]);
        let mut cursor = tiles.cursor();
        let e = float_edge(EdgeCursor::at(&mut cursor, 0));
        assert_eq!(e.x0, 127.0);
        assert!((e.y0 - 5.0).abs() <= 1.0 / 512.0);
        // A corner start belongs to the tile at the corner.
        let tiles = tiled(&[edge(127.0, 127.0, 140.0, 150.0)]);
        let (ox, oy, ..) = tile_at(&tiles, 0);
        assert_eq!([ox, oy], [127.0, 127.0]);
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
        let (ox, oy, ..) = tile_at(&tiles, 0);
        assert_eq!([ox, oy], [-127.0, -127.0]);
        for (e, q) in input.iter().zip(tiles.iter()) {
            assert!(distance(e, &q) <= 1.0 / 512.0, "decoded {q:?}");
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
