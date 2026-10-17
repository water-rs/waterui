//! Per-frame band bins over a raster's painter-ordered [`Item`] list.
//!
//! A [`FrameIndex`] is built once per `render_bands*` call and dropped
//! with it: flat prefix-offset tables give each `BAND_H`-row band the
//! item indices it must run — every structural item in original order,
//! plus each bounded draw whose device rows reach the band — and, for
//! each `Item::Draw`, the indices of its own edges that can cross the
//! band. Indices always name the original `items` slice and each draw's
//! own `edges`, so `PushFilter::end`, the top-capture plan and the
//! window/band split keep their original meaning.
//!
//! # Edge span conservatism
//!
//! `Band::draw` tests `ey = fl(e.y + dy)` with `dy = fl(oy − y0)` against
//! a band's rows. Under the standard model `fl(a op b) = (a op b)(1+δ)`,
//! `|δ| ≤ u = 2⁻²⁴` when the result is normal and `|fl(x) − x| ≤ 2⁻¹⁵⁰`
//! when it is subnormal:
//!
//! ```text
//! dy = (oy − y0)(1 + δ₁)
//! ey = (e.y + dy)(1 + δ₂)
//! ey − (e.y + oy − y0) = e.y·δ₂ + (oy − y0)(δ₁ + δ₂ + δ₁δ₂)
//! |err| ≤ u·|e.y| + γ₂·|oy − y0| ≤ γ₂·(|e.y| + |oy| + h),  γ₂ = 2u/(1−2u)
//! ```
//!
//! since `u < γ₂` and `|y0| ≤ h` for every surface and window row origin
//! (filter and capture windows stay inside `0..h`). Cancellation and
//! arbitrarily large offsets are covered: the bound is on magnitudes,
//! and `|dy| ≤ (|oy| + h)(1 + u)` independently of how close `e.y + dy`
//! lands to zero. The edge's device span `[min(e.y) + oy, max(e.y) + oy]`
//! is widened by `err = γ₂·(|e.y|max + |oy| + h) + 2⁻¹⁴⁹` — the additive
//! term covering both ops' subnormal cases — and the interval endpoints
//! are stepped one f64 ulp outward for the f64 arithmetic's own rounding.
//! Edges are finite by the lowering invariant (`flatten_edges` output is
//! validated finite), so no non-finite case exists to handle.
//!
//! # Expanded windows
//!
//! A filter scope's apron window or the capture run window spans
//! several bins: [`EdgeBins::query`] and [`FrameIndex::query_items`]
//! gather the covering bands' slices into reusable scratch and merge
//! them — sorted, deduplicated, back in original order. No whole-list
//! rescan.

use std::ops::Range;
use std::sync::atomic::AtomicU64;

use crate::render::lower::Item;
use crate::render::raster::{BAND_H, Edge};

/// Half an ulp of one round-to-nearest f32 rounding.
const U: f64 = f32::EPSILON as f64 / 2.0;

/// `γ₂ = 2u/(1 − 2u)`: the bound of `|δ₁ + δ₂ + δ₁δ₂|` over
/// `|δᵢ| ≤ u` — `2u + 4u² + … ≥ 2u + u²`.
const GAMMA2: f64 = 2.0 * U / (1.0 - 2.0 * U);

/// Both rounding steps' subnormal allowance: `2⁻¹⁴⁹ = 2 × 2⁻¹⁵⁰`.
const SUBNORMAL: f64 = 1.401_298_464_324_817e-45;

/// Heap metrics of one frame's transient band index, owned by the
/// render call. `render_bands*` reports into it; the render thread
/// reads it after shading.
#[derive(Debug, Default)]
pub struct FrameMeter {
    /// Exact heap capacity of the frame index's flat tables plus its
    /// construction temporaries, in bytes — the frame-local peak across
    /// the frame's render calls.
    pub index_bytes: AtomicU64,
    /// Simultaneously-live query-scratch bytes: expanded-window item and
    /// edge selections held at this instant across all workers.
    pub query_live: AtomicU64,
    /// `query_live`'s maximum over the frame's render calls.
    pub query_peak: AtomicU64,
}

/// The device rows `item` can draw in, clamped to `0..h`. Structural
/// items — isolates, filter scopes, captures and samples — cover the
/// whole surface: they run in every band.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    reason = "surface rows and mask coordinates are small integers, clamped before conversion"
)]
fn item_rows(item: &Item, h: usize) -> (usize, usize) {
    let clamp = |v: i64| (v.clamp(0, h as i64)) as usize;
    let mask_rows = |slot: &crate::render::glyph::GlyphSlot, oy: i64| {
        slot.get().map_or((0, h), |mask| {
            let top = oy + i64::from(mask.top);
            (clamp(top), clamp(top + i64::from(mask.h)))
        })
    };
    match item {
        Item::Draw { bbox, .. } => (bbox.y0 as usize, (bbox.y1 as usize).min(h)),
        Item::Shadow { bbox, .. } => (clamp(i64::from(bbox.y0)), clamp(i64::from(bbox.y1))),
        Item::Glyph { slot, y, .. } => mask_rows(slot, i64::from(*y)),
        Item::Silhouette { slot, .. } => mask_rows(slot, 0),
        Item::Project(item) => {
            let rows = item.placed.bounds.map(|v| v as usize);
            (rows[1].min(h), rows[3].min(h))
        }
        _ => (0, h),
    }
}

/// The bands a row range `[y0, y1)` reaches: `[y0/BAND_H, ⌈y1/BAND_H⌉)`
/// clipped to `0..bands`. `y0`/`y1` are already clamped to the surface,
/// so the conversions cannot overflow.
fn rows_bands((y0, y1): (usize, usize), bands: usize) -> Range<usize> {
    if y0 >= y1 {
        return 0..0;
    }
    (y0 / BAND_H).min(bands)..y1.div_ceil(BAND_H).min(bands)
}

/// The first and past-last band edge `e` — placed at origin `oy` — can
/// belong to, under the widened interval described in the module docs.
///
/// A band is needed exactly when its rows `[b·BAND_H, b·BAND_H + rows_b)`
/// can satisfy the draw's predicate, i.e. `b·BAND_H < hi` and
/// `b·BAND_H + rows_b > lo` for the widened `[lo, hi)`; `rows_b ≤ BAND_H`
/// makes the uniform `first = ⌊lo/BAND_H⌋` bound safe — a band below it
/// ends at or under `lo`, so the predicate's `min_ey < rows` fails.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the widened interval is clamped to the surface range in f64 \
              before the integer conversions, so every cast is in-range"
)]
fn edge_bands(e: &Edge, oy: f32, h: usize, bands: usize) -> (usize, usize) {
    let (y0, y1) = (f64::from(e.y0), f64::from(e.y1));
    let (emin, emax) = (y0.min(y1), y0.max(y1));
    // γ₂·(|e.y|max + |oy| + h) + subnormal allowance; `bound` below is an
    // upper bound of `|oy − y0|` over every band and window origin.
    let bound = f64::from(oy).abs() + h as f64;
    let err = GAMMA2.mul_add(y0.abs().max(y1.abs()) + bound, SUBNORMAL);
    // Outward rounding: one f64 ulp at each end covers the f64
    // arithmetic's own rounding of `emin/emax + oy` and the widened ends.
    let lo = (emin + f64::from(oy) - err).next_down();
    let hi = (emax + f64::from(oy) + err).next_up();
    // Clamp to the surface in f64 before converting: on an unclamped
    // huge `hi` the `as usize` cast saturates and the offset addition
    // could wrap. `hi.ceil()` lands the integer band boundary *after*
    // the upper endpoint, so `last` owns `⌈hi / BAND_H⌉` exactly — a
    // truncation here would drop the band the predicate still keeps.
    let (lo, hi) = (lo.clamp(0.0, h as f64), hi.clamp(0.0, h as f64));
    let first = (lo as usize) / BAND_H;
    let last = (hi.ceil() as usize).div_ceil(BAND_H);
    (first.min(last.min(bands)), last.min(bands))
}

/// One edge list's contribution to an [`EdgeBins`]: the item it belongs
/// to reaches bands `band0..band1`.
pub struct EdgeGroup<'a> {
    /// The draw's own edges, in original order.
    pub edges: &'a [Edge],
    /// The draw's device-space y origin.
    pub oy: f32,
    /// First band the owning item reaches.
    pub band0: usize,
    /// Past-last band the owning item reaches.
    pub band1: usize,
}

/// Flat `(group, band) → edge indices` bins over one or more edge
/// lists, sharing the draw-time placement arithmetic's band grid.
///
/// The pair space is group-major: group `g` owns pairs
/// `pair_base[g] .. pair_base[g] + span[g]`, one per band in its reach,
/// in ascending band order. Each pair's pool slice holds indices into
/// the group's own `edges`, in original order.
pub struct EdgeBins {
    /// Pair-space prefix offsets into `pool`; `pairs + 1` entries.
    off: Vec<usize>,
    /// Edge indices, per `(group, band)` pair.
    pool: Vec<usize>,
    /// Each group's first pair index.
    pair_base: Vec<usize>,
    /// Each group's first reachable band.
    band0: Vec<usize>,
    /// Each group's reachable band count.
    span: Vec<usize>,
    /// The build's peak live heap bytes: during the scatter, every
    /// final table plus the `fill` cursor is simultaneously live. The
    /// retained tables alone are always below it.
    build_peak: u64,
}

impl EdgeBins {
    /// Builds the bins for `groups` on a `bands`-band, `h`-row grid.
    pub fn build(groups: &[EdgeGroup<'_>], bands: usize, h: usize) -> Self {
        let mut pair_base = Vec::with_capacity(groups.len());
        let (mut band_first, mut span) = (
            Vec::with_capacity(groups.len()),
            Vec::with_capacity(groups.len()),
        );
        let mut pairs = 0usize;
        for group in groups {
            pair_base.push(pairs);
            band_first.push(group.band0);
            let n = group.band1.saturating_sub(group.band0);
            span.push(n);
            pairs += n;
        }
        let mut off = vec![0usize; pairs + 1];
        for (g, group) in groups.iter().enumerate() {
            for e in group.edges {
                let (ef, el) = edge_bands(e, group.oy, h, bands);
                for b in ef.max(group.band0)..el.min(group.band1) {
                    off[pair_base[g] + (b - group.band0) + 1] += 1;
                }
            }
        }
        for i in 0..pairs {
            off[i + 1] += off[i];
        }
        let mut pool = vec![0usize; off[pairs]];
        let mut fill = off.clone();
        for (g, group) in groups.iter().enumerate() {
            for (ei, e) in group.edges.iter().enumerate() {
                let (ef, el) = edge_bands(e, group.oy, h, bands);
                for b in ef.max(group.band0)..el.min(group.band1) {
                    let p = pair_base[g] + (b - group.band0);
                    pool[fill[p]] = ei;
                    fill[p] += 1;
                }
            }
        }
        let build_peak = ((off.capacity()
            + pool.capacity()
            + pair_base.capacity()
            + band_first.capacity()
            + span.capacity()
            + fill.capacity())
            * size_of::<usize>()) as u64;
        Self {
            off,
            pool,
            pair_base,
            band0: band_first,
            span,
            build_peak,
        }
    }

    /// The edge indices of `group` reaching `band`, in original order.
    /// Empty when the group does not reach the band.
    pub fn at(&self, group: usize, band: usize) -> &[usize] {
        let b0 = self.band0[group];
        if band < b0 || band >= b0 + self.span[group] {
            return &[];
        }
        let p = self.pair_base[group] + (band - b0);
        &self.pool[self.off[p]..self.off[p + 1]]
    }

    /// The union of `group`'s edge indices over the bands covering
    /// device rows `rows` — deduplicated, back in original edge order —
    /// appended to `out`.
    pub fn query(&self, group: usize, rows: (usize, usize), out: &mut Vec<usize>) {
        let b0 = self.band0[group];
        let covered = rows_bands(rows, b0 + self.span[group]);
        let start = out.len();
        for b in covered.start.max(b0)..covered.end {
            out.extend_from_slice(self.at(group, b));
        }
        // Vec::dedup drops the tail; on a subslice the compaction must
        // move and truncate explicitly.
        let tail = {
            let slice = &mut out[start..];
            slice.sort_unstable();
            let mut w = 0;
            for r in 1..slice.len() {
                if slice[r] != slice[w] {
                    w += 1;
                    slice[w] = slice[r];
                }
            }
            usize::from(!slice.is_empty()) + w
        };
        out.truncate(start + tail);
    }
}

/// A frame's per-band item and draw-edge bins, transient for the
/// `render_bands*` call that built it.
pub struct FrameIndex {
    /// The surface's band count: `⌈h / BAND_H⌉`.
    bands: usize,
    /// Band prefix offsets into `item_pool`; `bands + 1` entries.
    item_off: Vec<usize>,
    /// Original item indices per band, ascending.
    item_pool: Vec<usize>,
    /// `item index → draw rank`; `usize::MAX` for non-`Draw` items.
    draw_rank: Vec<usize>,
    /// The draws' edge bins, draw rank as group.
    edges: EdgeBins,
    /// The build's peak live heap bytes: the maximum over the
    /// construction stages of the tables and temporaries actually live
    /// at that stage — the item scatter (`item_off` + `item_pool` +
    /// item `fill`) versus the edge stage (all retained tables +
    /// `groups` + the edge scatter's own peak). The retained tables
    /// alone are always below it.
    build_peak: u64,
}

impl FrameIndex {
    /// Bins `items` over the band grid of a `h`-row surface.
    pub fn new(items: &[Item], h: usize) -> Self {
        let bands = h.div_ceil(BAND_H);
        let mut item_off = vec![0usize; bands + 1];
        for item in items {
            for b in rows_bands(item_rows(item, h), bands) {
                item_off[b + 1] += 1;
            }
        }
        for b in 0..bands {
            item_off[b + 1] += item_off[b];
        }
        let mut item_pool = vec![0usize; item_off[bands]];
        // Stage A — the item scatter: `item_off`, `item_pool` and the
        // fill cursor are simultaneously live.
        let stage_a = {
            let mut fill = item_off.clone();
            for (i, item) in items.iter().enumerate() {
                for b in rows_bands(item_rows(item, h), bands) {
                    item_pool[fill[b]] = i;
                    fill[b] += 1;
                }
            }
            ((item_off.capacity() + item_pool.capacity() + fill.capacity())
                * size_of::<usize>()) as u64
        };
        let (mut draw_rank, mut groups) = (vec![usize::MAX; items.len()], Vec::new());
        for (i, item) in items.iter().enumerate() {
            if let Item::Draw {
                edges,
                offset,
                bbox,
                ..
            } = item
            {
                draw_rank[i] = groups.len();
                let reach = rows_bands((bbox.y0 as usize, (bbox.y1 as usize).min(h)), bands);
                groups.push(EdgeGroup {
                    edges,
                    oy: offset[1],
                    band0: reach.start,
                    band1: reach.end,
                });
            }
        }
        // Stage B — the edge stage: every retained table, the `groups`
        // list and the edge scatter's own peak are live at once; the
        // item fill was already dropped.
        let edges = EdgeBins::build(&groups, bands, h);
        let stage_b = ((item_off.capacity() + item_pool.capacity() + draw_rank.capacity())
            * size_of::<usize>()) as u64
            + (groups.capacity() * size_of::<EdgeGroup<'_>>()) as u64
            + edges.build_peak;
        let build_peak = stage_a.max(stage_b);
        Self {
            bands,
            item_off,
            item_pool,
            draw_rank,
            edges,
            build_peak,
        }
    }

    /// The original item indices to run for band `b`: every structural
    /// item and the draws reaching it, in item order.
    pub fn items(&self, b: usize) -> &[usize] {
        &self.item_pool[self.item_off[b]..self.item_off[b + 1]]
    }

    /// The indices into draw `item`'s own `edges` reaching band `b`, in
    /// edge order. Empty for non-draw items and bands the draw misses.
    pub fn edges(&self, b: usize, item: usize) -> &[usize] {
        let rank = self.draw_rank[item];
        if rank == usize::MAX {
            return &[];
        }
        self.edges.at(rank, b)
    }

    /// The union of item indices in `range` over the bands covering
    /// device rows `rows` — deduplicated, in item order — appended to
    /// `out`. Structural items in `range` are always included.
    pub fn query_items(&self, range: Range<usize>, rows: (usize, usize), out: &mut Vec<usize>) {
        let start = out.len();
        for b in rows_bands(rows, self.bands) {
            out.extend(
                self.items(b)
                    .iter()
                    .copied()
                    .filter(|&i| range.contains(&i)),
            );
        }
        let tail = {
            let slice = &mut out[start..];
            slice.sort_unstable();
            let mut w = 0;
            for r in 1..slice.len() {
                if slice[r] != slice[w] {
                    w += 1;
                    slice[w] = slice[r];
                }
            }
            usize::from(!slice.is_empty()) + w
        };
        out.truncate(start + tail);
    }

    /// The union of draw `item`'s edge indices over the bands covering
    /// device rows `rows` — deduplicated, in edge order — appended to
    /// `out`.
    pub fn query_edges(&self, item: usize, rows: (usize, usize), out: &mut Vec<usize>) {
        let rank = self.draw_rank[item];
        if rank == usize::MAX {
            return;
        }
        self.edges.query(rank, rows, out);
    }

    /// The frame's peak live heap bytes for this index: the maximum
    /// over construction stages, which always covers the retained
    /// tables (`capacity()` sums) too.
    pub const fn heap_bytes(&self) -> u64 {
        self.build_peak
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pre-index band predicate, verbatim: an edge stays when its
    /// two-step float placement intersects the window's rows.
    #[expect(
        clippy::cast_precision_loss,
        reason = "mirrors the production predicate's row casts"
    )]
    fn kept(e: &Edge, oy: f32, y0: usize, rows: usize) -> bool {
        let dy = oy - y0 as f32;
        let ey0 = e.y0 + dy;
        let ey1 = e.y1 + dy;
        !(ey0.max(ey1) < 0.0 || ey0.min(ey1) >= rows as f32)
    }

    /// `kept` over a band grid of `bands` × `BAND_H` rows.
    fn kept_bands(e: &Edge, oy: f32, h: usize, bands: usize) -> Vec<usize> {
        (0..bands)
            .filter(|&b| kept(e, oy, b * BAND_H, (h - b * BAND_H).min(BAND_H)))
            .collect()
    }

    /// Bins a single edge list and checks, for every edge, that the bin
    /// contains the band whenever the reference predicate keeps it —
    /// and that every bin is sorted and unique.
    fn assert_bins_cover(edges: &[Edge], oy: f32, h: usize) {
        let bands = h.div_ceil(BAND_H);
        let index = EdgeBins::build(
            &[EdgeGroup {
                edges,
                oy,
                band0: 0,
                band1: bands,
            }],
            bands,
            h,
        );
        for b in 0..bands {
            let bin = index.at(0, b);
            assert!(bin.windows(2).all(|w| w[0] < w[1]), "bin {b} sorted unique");
        }
        for (ei, e) in edges.iter().enumerate() {
            for b in kept_bands(e, oy, h, bands) {
                assert!(
                    index.at(0, b).contains(&ei),
                    "edge {ei} {e:?} kept at band {b} but not binned"
                );
            }
        }
    }

    /// Edges spanning directions, magnitudes and translations over
    /// full and partial-band surfaces, including f32 neighbours of band
    /// boundaries and offsets that cancel against edge y.
    #[test]
    fn bins_cover_the_old_band_predicate() {
        let mut edges = Vec::new();
        for &len in &[0.5f32, 3.0, 15.9, 16.0, 16.1, 61.0, 1e4, -1e4] {
            for &top in &[0.0f32, 15.999_999, 16.0, 16.000_002, -0.5, 63.75] {
                edges.push(Edge {
                    x0: 1.0,
                    y0: top,
                    x1: 9.0,
                    y1: top + len,
                });
                edges.push(Edge {
                    x0: 1.0,
                    y0: top + len,
                    x1: 9.0,
                    y1: top,
                });
            }
        }
        edges.push(Edge {
            x0: 0.0,
            y0: 16.0_f32.next_down(),
            x1: 1.0,
            y1: 16.0_f32.next_up(),
        });
        edges.push(Edge {
            x0: 0.0,
            y0: -1.0,
            x1: 1.0,
            y1: 0.0,
        });
        // Cancellation: an offset that lands e.y + oy near zero and near
        // each band edge, plus one very large offset.
        for &oy in &[0.0f32, -0.5, -64.0, 1e3, 33.333_336, -1e6] {
            for &h in &[16usize, 17, 64, 901] {
                assert_bins_cover(&edges, oy, h);
            }
        }
    }

    /// Expanded windows: the merged query is a superset of what the
    /// predicate keeps over the window rows, sorted and unique, for a
    /// nonaligned window and a window clipped to the surface edge.
    #[test]
    #[expect(
        clippy::cast_precision_loss,
        reason = "test edge coordinates stay well under 2^24"
    )]
    fn window_queries_merge_bins_in_order() {
        let edges: Vec<Edge> = (0..60)
            .map(|i| Edge {
                x0: i as f32,
                y0: (i as f32 * 3.7) % 90.0 - 20.0,
                x1: i as f32,
                y1: (i as f32 * 3.7) % 90.0 - 20.0 + (i as f32 % 37.0) - 5.0,
            })
            .collect();
        let h: usize = 128;
        let bands = h.div_ceil(BAND_H);
        let index = EdgeBins::build(
            &[EdgeGroup {
                edges: &edges,
                oy: 7.5,
                band0: 0,
                band1: bands,
            }],
            bands,
            h,
        );
        for rows in [(7, 49), (0, 5), (120, 128), (3, 127)] {
            let mut sel = Vec::new();
            index.query(0, rows, &mut sel);
            assert!(
                sel.windows(2).all(|w| w[0] < w[1]),
                "{rows:?} sorted unique"
            );
            for (ei, e) in edges.iter().enumerate() {
                if kept(e, 7.5, rows.0, rows.1 - rows.0) {
                    assert!(sel.contains(&ei), "edge {ei} missed in {rows:?}");
                }
            }
        }
    }

    /// `item_rows`/`query_items` keep structural items in range and drop
    /// draws the window misses.
    #[test]
    fn item_bins_track_bounds_and_scopes() {
        use crate::render::lower::DrawBox;
        let draw = |y0: u16, y1: u16| Item::Draw {
            edges: Vec::new().into(),
            offset: [0.0, 0.0],
            bbox: DrawBox {
                x0: 0,
                y0,
                x1: 10,
                y1,
            },
            rule: cherenkov::FillRule::NonZero,
            paint: crate::render::paint::PaintData::Solid([0.0; 4]),
            clip: None,
            swept: false,
        };
        let items = vec![
            Item::PushIsolate {
                space: cherenkov::BlendSpace::Linear,
            },
            draw(0, 20),
            draw(64, 80),
            Item::PopIsolate {
                opacity: 1.0,
                blend: (cherenkov::BlendMode::Normal, cherenkov::BlendSpace::Linear),
                clip: None,
            },
        ];
        let h = 96;
        let index = FrameIndex::new(&items, h);
        // Band 1 (rows 16..32): structural items plus the first draw.
        assert_eq!(index.items(1), &[0, 1, 3]);
        // Band 5 (rows 80..96): only structural — the second draw's box
        // is 64..80, which touches bands 4..5, not 5.
        assert_eq!(index.items(5), &[0, 3]);
        // A windowed query over rows 18..70 restricted to items 1..3
        // keeps both draws (bands 1..5 covered) and no structural items.
        let mut sel = Vec::new();
        index.query_items(1..3, (18, 70), &mut sel);
        assert_eq!(sel, &[1, 2]);
        // A window covering rows 0..16: structural items still appear.
        let mut sel = Vec::new();
        index.query_items(0..4, (0, 16), &mut sel);
        assert_eq!(sel, &[0, 1, 3]);
    }
}