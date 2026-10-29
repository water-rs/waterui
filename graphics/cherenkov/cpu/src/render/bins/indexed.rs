//! Whole-frame bins for desktop and mobile rasterization.

use super::item_rows;
use crate::render::lower::Item;
use crate::render::raster::BAND_H;
use std::ops::Range;

pub type SelectionBuffer = Vec<usize>;
pub type Selection<'a> = std::iter::Copied<std::slice::Iter<'a, usize>>;

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
    fn band(&self, b: usize) -> &[usize] {
        &self.item_pool[self.item_off[b]..self.item_off[b + 1]]
    }

    /// The union of item indices in `range` over the bands covering
    /// device rows `rows` — deduplicated, in item order — appended to
    /// `out`. Structural items in `range` are always included.
    pub fn query_items(&self, range: Range<usize>, rows: (usize, usize), out: &mut Vec<usize>) {
        let start = out.len();
        for b in rows_bands(rows, self.bands) {
            out.extend(self.band(b).iter().copied().filter(|&i| range.contains(&i)));
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

    pub fn items<'a>(&'a self, b: usize, _items: &'a [Item]) -> Selection<'a> {
        self.band(b).iter().copied()
    }

    pub fn items_after<'a>(&'a self, b: usize, _items: &'a [Item], start: usize) -> Selection<'a> {
        let all = self.band(b);
        all[all.partition_point(|&i| i < start)..].iter().copied()
    }

    #[expect(
        clippy::unused_self,
        reason = "the streaming strategy reads surface height for this selection"
    )]
    pub fn selected<'a>(&self, buffer: &'a SelectionBuffer, _items: &'a [Item]) -> Selection<'a> {
        buffer.iter().copied()
    }

    pub const fn heap_bytes(&self) -> u64 {
        ((self.item_off.capacity() + self.item_pool.capacity() + self.counts.capacity())
            * size_of::<usize>()) as u64
    }
}
