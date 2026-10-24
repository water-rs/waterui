//! Painter-ordered item bins. Each geometry owns its exact interval index.

use std::ops::Range;
use std::sync::atomic::AtomicU64;

use crate::render::lower::Item;
use crate::render::raster::BAND_H;

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
pub(super) fn item_rows(item: &Item, h: usize) -> (usize, usize) {
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

/// Per-band item bins, rebuilt in renderer-owned reusable storage.
#[derive(Default)]
pub struct FrameIndex {
    /// The surface's band count: `⌈h / BAND_H⌉`.
    bands: usize,
    /// Band prefix offsets into `item_pool`; `bands + 1` entries.
    item_off: Vec<usize>,
    /// Original item indices per band, ascending.
    item_pool: Vec<usize>,
    /// The shared count buffer: all-zero at every rebuild boundary —
    /// counting writes it up, the scatter reads it back down to zero,
    /// so it never needs a clearing pass.
    counts: Vec<usize>,
    verticals: Option<[usize; 2]>,
}

impl FrameIndex {
    /// Vertical scratch bound for draws; sloped-only draws still need rows.
    pub fn coverage_capacity(&self, windowed: bool) -> Option<usize> {
        self.verticals.map(|counts| counts[usize::from(windowed)])
    }

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
        self.verticals = None;
        // One item walk: per-band counts and vertical scratch bounds.
        for item in items {
            for b in rows_bands(item_rows(item, h), bands) {
                self.counts[b] += 1;
            }
            if let Item::Draw { edges, .. } = item {
                let verticals = self.verticals.get_or_insert([0; 2]);
                verticals[0] = verticals[0].max(edges.vertical_capacity(false));
                verticals[1] = verticals[1].max(edges.vertical_capacity(true));
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
    }

    /// The original item indices to run for band `b`: every structural
    /// item and the draws reaching it, in item order.
    pub fn items(&self, b: usize) -> &[usize] {
        &self.item_pool[self.item_off[b]..self.item_off[b + 1]]
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

    pub const fn heap_bytes(&self) -> u64 {
        ((self.item_off.capacity() + self.item_pool.capacity() + self.counts.capacity())
            * size_of::<usize>()) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::render::tiles::EdgeTiles;

    #[test]
    fn item_bins_track_bounds_and_scopes() {
        use crate::render::lower::DrawBox;
        let draw = |y0: u16, y1: u16| Item::Draw {
            edges: EdgeTiles::new(
                &mut crate::render::tiles::EdgeScratch::default(),
                std::iter::empty(),
            ),
            offset: crate::render::tiles::Fixed::new([0.0, 0.0]),
            bbox: DrawBox {
                x0: 0,
                y0,
                x1: 10,
                y1,
            },
            paint: crate::render::paint::PaintData::Solid([0.0; 4]),
            clip: None,
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
