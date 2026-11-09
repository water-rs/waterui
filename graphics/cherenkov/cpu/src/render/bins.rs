//! Painter-ordered item bins. Each geometry owns its exact interval index.

use std::sync::atomic::AtomicU64;

use crate::render::lower::Item;

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
    /// Simultaneously live clip samples, compilers and pool bookkeeping.
    pub clip_live: AtomicU64,
    /// Clip working storage peak, included in the surface memory report.
    pub clip_peak: AtomicU64,
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

#[cfg(not(feature = "bounded-bins"))]
mod indexed;
#[cfg(feature = "bounded-bins")]
mod streaming;
#[cfg(not(feature = "bounded-bins"))]
pub use indexed::{FrameIndex, Selection, SelectionBuffer};
#[cfg(feature = "bounded-bins")]
pub use streaming::{FrameIndex, Selection, SelectionBuffer};

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
        assert_eq!(index.items(1, &items).collect::<Vec<_>>(), &[0, 1, 3]);
        // Band 5 (rows 80..96): only structural — the second draw's box
        // is 64..80, which touches bands 4..5, not 5.
        assert_eq!(index.items(5, &items).collect::<Vec<_>>(), &[0, 3]);
        // A windowed query over rows 18..70 restricted to items 1..3
        // keeps both draws (bands 1..5 covered) and no structural items.
        let mut sel = SelectionBuffer::default();
        index.query_items(1..3, (18, 70), &mut sel);
        assert_eq!(index.selected(&sel, &items).collect::<Vec<_>>(), &[1, 2]);
        // A window covering rows 0..16: structural items still appear.
        let mut sel = SelectionBuffer::default();
        index.query_items(0..4, (0, 16), &mut sel);
        assert_eq!(index.selected(&sel, &items).collect::<Vec<_>>(), &[0, 1, 3]);
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
        assert_eq!(index.items(1, &items2).collect::<Vec<_>>(), &[0, 1, 2]);
        assert_eq!(index.items(5, &items2).collect::<Vec<_>>(), &[1, 2]);
        // A smaller surface rebuilds in place over fewer bands.
        index.rebuild(&items, 32);
        assert_eq!(index.items(1, &items).collect::<Vec<_>>(), &[0, 1, 3]);
        // And a larger one again — no stale band survives either way.
        index.rebuild(&items2, 96);
        assert_eq!(index.items(1, &items2).collect::<Vec<_>>(), &[0, 1, 2]);
        assert_eq!(index.items(5, &items2).collect::<Vec<_>>(), &[1, 2]);
    }
}
