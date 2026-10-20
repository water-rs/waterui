//! Retained flattened geometry: frame-space edges quantized to 1/256 px
//! and grouped by fixed 127 × 127 px tiles.
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
//! Decoding is `origin + q * (1/256)` in `f32`: the product is exact (a
//! power of two) and the sum is exact at every coordinate this backend
//! produces, so every consumer reads decoded `f32` edges and keeps its
//! own arithmetic.

use std::ops::Range;

use super::raster::Edge;

/// A tile's side in pixels.
const TILE: f64 = 127.0;
/// Quantization units per pixel: one unit is 1/256 px and an `i16`
/// reaches ±128 px from its tile's origin.
const UNIT: f64 = 256.0;
/// `UNIT` as the decode multiplier.
const DECODE: f32 = 1.0 / 256.0;

/// A quantized edge: endpoints in 1/256 px relative to its tile's origin.
#[derive(Clone, Copy, Debug)]
struct QEdge {
    x0: i16,
    y0: i16,
    x1: i16,
    y1: i16,
}

/// One tile of the frame grid and the `edges` range it owns.
#[derive(Debug)]
struct Tile {
    /// The tile's origin: an integer multiple of 127 on each axis.
    ox: f32,
    oy: f32,
    /// `edges[start..end]` belong to this tile.
    start: u32,
    end: u32,
}

impl Tile {
    const fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    /// `origin + q * (1/256)` in `f32`, one rounding per coordinate.
    fn decode(&self, q: QEdge) -> Edge {
        Edge {
            x0: f32::from(q.x0).mul_add(DECODE, self.ox),
            y0: f32::from(q.y0).mul_add(DECODE, self.oy),
            x1: f32::from(q.x1).mul_add(DECODE, self.ox),
            y1: f32::from(q.y1).mul_add(DECODE, self.oy),
        }
    }
}

/// A draw's flattened edges in retained, tile-local fixed point.
///
/// The quantization step runs where the `f32` form was built — after the
/// winding sweep — so every retained edge is `i16` at 1/256 px and only
/// transient edge lists stay in `f32`.
#[derive(Debug)]
pub struct EdgeTiles {
    /// Tiles by edge range: contiguous and covering `0..edges.len()`.
    tiles: Box<[Tile]>,
    /// Quantized edges, grouped by tile.
    edges: Box<[QEdge]>,
}

/// The tile of the frame grid containing `(x, y)` — floor to a 127 px
/// boundary, so a point exactly on a boundary belongs to the tile
/// starting there.
fn tile_of(x: f64, y: f64) -> (f64, f64) {
    ((x / TILE).floor() * TILE, (y / TILE).floor() * TILE)
}

/// The nearest unit of 1/256 px, ties rounded up: the tile origins are
/// multiples of 127 × 256 units, so the same frame point rounds to the
/// same unit from every tile, and the ±32512-unit range can never reach
/// the `i16` limits.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the float-to-int cast is the quantization rule; inputs stay in ±32512"
)]
#[expect(
    clippy::missing_const_for_fn,
    reason = "f64 mul_add and floor are not const-stable"
)]
fn quantize(v: f64) -> i16 {
    v.mul_add(UNIT, 0.5).floor() as i16
}

/// Emits `e`'s quantized pieces to `emit` as `(tile origin, piece)`
/// pairs: the part inside each crossed range boundary becomes its own
/// edge under the tile holding its start.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a tile's origin is a multiple of 128, exact in f32"
)]
fn split(e: &Edge, mut emit: impl FnMut([f32; 2], QEdge)) {
    let (mut x0, mut y0) = (f64::from(e.x0), f64::from(e.y0));
    let (x1, y1) = (f64::from(e.x1), f64::from(e.y1));
    loop {
        let (tx, ty) = tile_of(x0, y0);
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
            emit([tx as f32, ty as f32], q);
        }
        if inside {
            break;
        }
        x0 = px;
        y0 = py;
    }
}

impl EdgeTiles {
    /// Quantizes `edges` — finite frame-space `f32` segments — into
    /// tile-local fixed point.
    pub fn new(edges: &[Edge]) -> Self {
        // (tile origin, quantized piece) pairs in emission order.
        let mut pieces = Vec::with_capacity(edges.len());
        for e in edges {
            split(e, |origin, q| pieces.push((origin, q)));
        }
        // Group by tile; the stable sort keeps each tile's pieces in
        // emission order.
        pieces.sort_by_key(|(origin, _)| (origin[0].to_bits(), origin[1].to_bits()));
        let mut tiles: Vec<Tile> = Vec::new();
        let mut quantized = Vec::with_capacity(pieces.len());
        for (origin, q) in pieces {
            let last = tiles.last_mut();
            let tile = match last {
                Some(t)
                    if t.ox.to_bits() == origin[0].to_bits()
                        && t.oy.to_bits() == origin[1].to_bits() =>
                {
                    t
                }
                _ => {
                    tiles.push(Tile {
                        ox: origin[0],
                        oy: origin[1],
                        start: u32::try_from(quantized.len()).expect("a draw's edges fit u32"),
                        end: 0,
                    });
                    tiles.last_mut().expect("just pushed")
                }
            };
            quantized.push(q);
            tile.end = u32::try_from(quantized.len()).expect("a draw's edges fit u32");
        }
        Self {
            tiles: tiles.into(),
            edges: quantized.into(),
        }
    }

    /// The edge count, split pieces included.
    pub fn len(&self) -> usize {
        self.edges.len()
    }

    /// Whether no edge survived quantization.
    pub fn is_empty(&self) -> bool {
        self.edges.is_empty()
    }

    /// Decoded frame-space edge `i`.
    pub fn edge(&self, i: usize) -> Edge {
        let tile = &self.tiles[self.tiles.partition_point(|t| t.end as usize <= i)];
        tile.decode(self.edges[i])
    }

    /// The decoded edges in storage order.
    pub fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        self.tiles.iter().flat_map(|tile| {
            self.edges[tile.range()]
                .iter()
                .map(move |&q| tile.decode(q))
        })
    }

    /// The real heap size: 8 bytes per edge plus the tile table.
    pub fn heap_bytes(&self) -> u64 {
        (self.edges.len() * size_of::<QEdge>() + self.tiles.len() * size_of::<Tile>()) as u64
    }
}

/// An edge list the band bins index: retained tile-local geometry
/// ([`EdgeTiles`]) or transient `f32` edges — clip coverage masks,
/// silhouette captures — that are never retained (`[Edge]`).
pub trait EdgeSource: Sync {
    /// The list's edge count.
    fn len(&self) -> usize;

    /// Decoded edge `i`.
    fn edge(&self, i: usize) -> Edge;

    /// The decoded edges in order.
    fn iter(&self) -> impl Iterator<Item = Edge> + '_;
}

impl EdgeSource for EdgeTiles {
    fn len(&self) -> usize {
        Self::len(self)
    }

    fn edge(&self, i: usize) -> Edge {
        Self::edge(self, i)
    }

    fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        Self::iter(self)
    }
}

impl EdgeSource for [Edge] {
    fn len(&self) -> usize {
        <[Edge]>::len(self)
    }

    fn edge(&self, i: usize) -> Edge {
        self[i]
    }

    fn iter(&self) -> impl Iterator<Item = Edge> + '_ {
        <[Edge]>::iter(self).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn edge(x0: f32, y0: f32, x1: f32, y1: f32) -> Edge {
        Edge { x0, y0, x1, y1 }
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
        let tiles = EdgeTiles::new(&input);
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
        let tiles = EdgeTiles::new(&[edge(10.0, 10.0, 500.0, 380.0)]);
        assert!(tiles.len() > 2, "expected several pieces: {tiles:?}");
        // Every piece's decoded start sits inside its own tile and every
        // decoded coordinate stays within ±127 px of the origin.
        for tile in &tiles.tiles {
            for &q in &tiles.edges[tile.range()] {
                let e = tile.decode(q);
                assert!((0.0..=127.0).contains(&(e.x0 - tile.ox)));
                assert!((0.0..=127.0).contains(&(e.y0 - tile.oy)));
                for rel in [e.x1 - tile.ox, e.y1 - tile.oy] {
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
        let tiles = EdgeTiles::new(&[edge(127.0, 5.0, 140.0, 60.0)]);
        assert_eq!(tiles.tiles.len(), 1);
        assert_eq!([tiles.tiles[0].ox, tiles.tiles[0].oy], [127.0, 0.0]);
        let e = tiles.edge(0);
        assert_eq!(e.x0, 127.0);
        assert!((e.y0 - 5.0).abs() <= 1.0 / 512.0);
        // A corner start belongs to the tile at the corner.
        let tiles = EdgeTiles::new(&[edge(127.0, 127.0, 140.0, 150.0)]);
        assert_eq!([tiles.tiles[0].ox, tiles.tiles[0].oy], [127.0, 127.0]);
    }

    /// Negative frame coordinates tile and decode like positive ones:
    /// floor to the 127 px grid, not truncation toward zero.
    #[test]
    fn negative_frame_coordinates() {
        let input = [
            edge(-50.0, -50.0, -20.0, -60.0),
            edge(-20.0, -10.0, -5.0, -60.0),
        ];
        let tiles = EdgeTiles::new(&input);
        // Both edges start in the (-127, -127) tile and stay in range.
        assert_eq!(tiles.len(), 2);
        assert_eq!([tiles.tiles[0].ox, tiles.tiles[0].oy], [-127.0, -127.0]);
        for (e, q) in input.iter().zip(tiles.iter()) {
            assert!(distance(e, &q) <= 1.0 / 512.0, "decoded {q:?}");
        }
    }

    /// A placement offset moves the decoded edges, not the retained
    /// geometry: the same `Arc` serves every translation.
    #[test]
    fn translation_reuses_the_same_geometry() {
        let geometry = Arc::new(EdgeTiles::new(&[
            edge(1.0, 2.0, 40.0, 60.0),
            edge(40.0, 60.0, 90.0, 12.5),
        ]));
        let moved = geometry.clone();
        assert!(Arc::ptr_eq(&geometry, &moved));
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
        let tiles = EdgeTiles::new(&edges);
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
