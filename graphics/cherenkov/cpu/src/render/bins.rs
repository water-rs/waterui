//! Per-frame band bins over a raster's painter-ordered [`Item`] list.
//!
//! A [`FrameIndex`] is rebuilt per `render_bands*` call and reused across
//! sequential calls and frames. Flat prefix-offset tables
//! give each `BAND_H`-row band the item indices it must run — every structural item in original order,
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

/// Heap metrics of a frame's index and expanded-window query scratch.
/// `render_bands*` records the index before sharing the meter with workers;
/// the render thread reads the combined peak after shading.
#[derive(Debug, Default)]
pub struct FrameMeter {
    /// Current index allocation, fixed while workers run.
    pub index_bytes: u64,
    /// Peak simultaneously live index and query-scratch allocation.
    pub peak: AtomicU64,
    /// Simultaneously-live query-scratch bytes: expanded-window item and
    /// edge selections held at this instant across all workers.
    pub query_live: AtomicU64,
    /// `query_live`'s maximum over the frame's render calls.
    pub query_peak: AtomicU64,
}

impl FrameMeter {
    /// Records the rebuilt index before any workers borrow the meter.
    pub fn record_index(&mut self, bytes: u64) {
        self.index_bytes = bytes;
        self.peak
            .fetch_max(bytes, std::sync::atomic::Ordering::Relaxed);
    }
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
/// to reaches bands `band0..band1`. A rebuild-time descriptor — never
/// retained.
#[derive(Clone, Copy)]
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
///
/// Every table keeps its capacity across [`EdgeBins::rebuild`]s: the
/// renderer reuses the storage across frames instead of
/// re-allocating per call.
#[derive(Default)]
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
    /// Each group's first index into `edge_spans`.
    span_off: Vec<usize>,
    /// Each edge's `[first, last)` band span, computed once per edge
    /// per rebuild and reused verbatim by the scatter pass.
    edge_spans: Vec<(usize, usize)>,
}

impl EdgeBins {
    /// Rebuilds the bins for `groups` over the `bands`-band, `h`-row
    /// grid, retaining all table capacity. `groups` is a cloneable
    /// view (it is iterated once per pass, nothing borrowed is
    /// retained). `counts` is all-zero on entry and restored to
    /// all-zero on exit: the scatter writes at `off[p+1] − counts[p]`
    /// and decrements, so every counted reference lands exactly once.
    pub fn rebuild<'a, I>(&mut self, groups: I, counts: &mut Vec<usize>, bands: usize, h: usize)
    where
        I: Iterator<Item = EdgeGroup<'a>> + Clone,
    {
        // Group metadata: reach, pair prefix and span prefix.
        self.pair_base.clear();
        self.band0.clear();
        self.span.clear();
        self.span_off.clear();
        let (mut pairs, mut edges_total) = (0usize, 0usize);
        for g in groups.clone() {
            self.pair_base.push(pairs);
            self.band0.push(g.band0);
            self.span.push(g.band1.saturating_sub(g.band0));
            self.span_off.push(edges_total);
            edges_total += g.edges.len();
            pairs += g.band1.saturating_sub(g.band0);
        }
        // Each edge's widened band span, once per edge per rebuild.
        self.edge_spans.resize(edges_total, (0, 0));
        for (g, group) in groups.clone().enumerate() {
            for (ei, e) in group.edges.iter().enumerate() {
                self.edge_spans[self.span_off[g] + ei] = edge_bands(e, group.oy, h, bands);
            }
        }
        // Count in pair space. `counts` keeps its high-water length —
        // the `counts[..pairs]` prefix is active and all-zero on entry
        // (the tail stayed zero from the last scatter).
        counts.resize(counts.len().max(pairs), 0);
        for (g, group) in groups.clone().enumerate() {
            for ei in 0..group.edges.len() {
                let (ef, el) = self.edge_spans[self.span_off[g] + ei];
                for b in ef.max(group.band0)..el.min(group.band1) {
                    counts[self.pair_base[g] + (b - group.band0)] += 1;
                }
            }
        }
        // Prefix sums over the active prefix: read `counts`, write
        // `off` — no clearing pass.
        self.off.resize(pairs + 1, 0);
        self.off[0] = 0;
        let mut total = 0;
        for (p, &c) in counts[..pairs].iter().enumerate() {
            total += c;
            self.off[p + 1] = total;
        }
        self.pool.resize(total, 0);
        // Scatter at `off[p+1] − counts[p]` — the pair's end minus its
        // remaining count — filling each slice forward from its start
        // in original order. Every counted reference lands exactly
        // once, so `counts` returns to all-zero by construction.
        for (g, group) in groups.enumerate() {
            for ei in 0..group.edges.len() {
                let (ef, el) = self.edge_spans[self.span_off[g] + ei];
                for b in ef.max(group.band0)..el.min(group.band1) {
                    let p = self.pair_base[g] + (b - group.band0);
                    self.pool[self.off[p + 1] - counts[p]] = ei;
                    counts[p] -= 1;
                }
            }
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

    /// The retained heap capacity of every table and scratch Vec.
    pub const fn heap_bytes(&self) -> u64 {
        ((self.off.capacity()
            + self.pool.capacity()
            + self.pair_base.capacity()
            + self.band0.capacity()
            + self.span.capacity()
            + self.span_off.capacity())
            * size_of::<usize>()
            + self.edge_spans.capacity() * size_of::<(usize, usize)>()) as u64
    }
}

/// Per-band item and edge bins, rebuilt in renderer-owned reusable storage.
#[derive(Default)]
pub struct FrameIndex {
    /// The surface's band count: `⌈h / BAND_H⌉`.
    bands: usize,
    /// Band prefix offsets into `item_pool`; `bands + 1` entries.
    item_off: Vec<usize>,
    /// Original item indices per band, ascending.
    item_pool: Vec<usize>,
    /// The `Item::Draw` items' indices, in item order — the
    /// `EdgeBins` groups' items.
    draws: Vec<usize>,
    /// `item index → draw rank`; `usize::MAX` for non-`Draw` items.
    draw_rank: Vec<usize>,
    /// The shared count buffer: all-zero at every rebuild boundary —
    /// counting writes it up, the scatter reads it back down to zero,
    /// so it never needs a clearing pass.
    counts: Vec<usize>,
    /// The draws' edge bins, draw rank as group.
    edges: EdgeBins,
}

impl FrameIndex {
    /// Rebuilds the bins for `items` over the band grid of an `h`-row
    /// surface. Called once per `render_bands*` before the parallel
    /// band pass reads the tables.
    pub fn rebuild(&mut self, items: &[Item], h: usize) {
        let bands = h.div_ceil(BAND_H);
        self.bands = bands;
        debug_assert!(self.counts.iter().all(|&c| c == 0));
        // `counts` keeps its high-water length — `counts[..bands]` is
        // the active prefix and all-zero on entry (the tail stayed
        // zero from the last scatter).
        self.counts.resize(self.counts.len().max(bands), 0);
        self.draws.clear();
        self.draw_rank.resize(items.len(), usize::MAX);
        // One item walk: per-band counts, draw ranks and the draw list.
        for (i, item) in items.iter().enumerate() {
            for b in rows_bands(item_rows(item, h), bands) {
                self.counts[b] += 1;
            }
            if let Item::Draw { .. } = item {
                self.draw_rank[i] = self.draws.len();
                self.draws.push(i);
            } else {
                self.draw_rank[i] = usize::MAX;
            }
        }
        // Prefix sums over the active prefix: read `counts`, write
        // `item_off` — no clearing pass.
        self.item_off.resize(bands + 1, 0);
        self.item_off[0] = 0;
        let mut total = 0;
        for (b, &c) in self.counts[..bands].iter().enumerate() {
            total += c;
            self.item_off[b + 1] = total;
        }
        self.item_pool.resize(total, 0);
        // Scatter at `item_off[b+1] − counts[b]` — the band's end minus
        // its remaining count — filling each slice forward from its
        // start in original order. Every counted reference lands
        // exactly once, so `counts` returns to all-zero by construction.
        for (i, item) in items.iter().enumerate() {
            for b in rows_bands(item_rows(item, h), bands) {
                self.item_pool[self.item_off[b + 1] - self.counts[b]] = i;
                self.counts[b] -= 1;
            }
        }
        // A cloneable view of the draws' placements — nothing borrowed
        // is retained, no `Vec<EdgeGroup>` exists per rebuild.
        self.edges.rebuild(
            self.draws.iter().map(|&i| {
                let Item::Draw {
                    edges,
                    offset,
                    bbox,
                    ..
                } = &items[i]
                else {
                    unreachable!("draws only names Item::Draw")
                };
                let reach = rows_bands((bbox.y0 as usize, (bbox.y1 as usize).min(h)), bands);
                EdgeGroup {
                    edges,
                    oy: offset[1],
                    band0: reach.start,
                    band1: reach.end,
                }
            }),
            &mut self.counts,
            bands,
            h,
        );
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

    /// The retained heap capacity of every table and scratch Vec —
    /// reported under `Engine::memory()`'s band-buffers category.
    pub const fn heap_bytes(&self) -> u64 {
        ((self.item_off.capacity()
            + self.item_pool.capacity()
            + self.draws.capacity()
            + self.draw_rank.capacity()
            + self.counts.capacity())
            * size_of::<usize>()) as u64
            + self.edges.heap_bytes()
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

    /// Rebuilds `index` in place for a single edge list and checks, for
    /// every edge, that the bin contains the band whenever the
    /// reference predicate keeps it — and that every bin is sorted and
    /// unique. Reusing one index across calls exercises the retained
    /// storage's reset between shapes.
    fn assert_bins_cover(
        index: &mut EdgeBins,
        counts: &mut Vec<usize>,
        edges: &[Edge],
        oy: f32,
        h: usize,
    ) {
        let bands = h.div_ceil(BAND_H);
        index.rebuild(
            std::iter::once(EdgeGroup {
                edges,
                oy,
                band0: 0,
                band1: bands,
            }),
            counts,
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
        let mut index = EdgeBins::default();
        let mut counts = Vec::new();
        for &oy in &[0.0f32, -0.5, -64.0, 1e3, 33.333_336, -1e6] {
            for &h in &[16usize, 17, 64, 901] {
                assert_bins_cover(&mut index, &mut counts, &edges, oy, h);
            }
        }
        // Smaller and larger band counts plus a different edge list on
        // the same retained storage.
        assert_bins_cover(&mut index, &mut counts, &edges[..40], -3.0, 48);
        assert_bins_cover(&mut index, &mut counts, &edges, -3.0, 16);
        assert_bins_cover(&mut index, &mut counts, &edges[..40], -3.0, 901);
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
        let mut index = EdgeBins::default();
        index.rebuild(
            std::iter::once(EdgeGroup {
                edges: &edges,
                oy: 7.5,
                band0: 0,
                band1: bands,
            }),
            &mut Vec::new(),
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
        let mut index = FrameIndex::default();
        index.rebuild(&items, h);
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
        // Rebuild the same retained index over a different item shape:
        // the first draw reaches bands 0..4 (rows 0..50), the second
        // 0..6 (rows 0..90), and only item 1 is structural.
        let items2 = vec![
            draw(0, 50),
            Item::PushIsolate {
                space: cherenkov::BlendSpace::Linear,
            },
            draw(0, 90),
        ];
        index.rebuild(&items2, 96);
        assert_eq!(index.items(1), &[0, 1, 2]);
        assert_eq!(index.items(5), &[1, 2]);
        // A smaller surface rebuilds in place over fewer bands.
        index.rebuild(&items, 32);
        assert_eq!(index.items(1), &[0, 1, 3]);
        // And a larger one again — no stale band survives either way.
        index.rebuild(&items2, 96);
        assert_eq!(index.items(1), &[0, 1, 2]);
        assert_eq!(index.items(5), &[1, 2]);
    }
}
