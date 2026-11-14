//! Firmware bins scan retained items without materializing selections.

use super::item_rows;
use crate::render::lower::Item;
use crate::render::raster::BAND_H;
use std::ops::Range;

#[derive(Default)]
pub struct FrameIndex {
    height: usize,
}

#[derive(Default)]
pub struct SelectionBuffer {
    range: Range<usize>,
    rows: (usize, usize),
}

impl SelectionBuffer {
    pub const fn clear(&mut self) {
        self.range = 0..0;
    }

    #[expect(
        clippy::unused_self,
        reason = "same capacity interface as the indexed selection buffer"
    )]
    pub const fn capacity(&self) -> usize {
        0
    }
}

pub struct Selection<'a> {
    items: &'a [Item],
    range: Range<usize>,
    rows: (usize, usize),
    height: usize,
}

impl Iterator for Selection<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        self.range.find(|&i| {
            let (top, bottom) = item_rows(&self.items[i], self.height);
            top < bottom && top < self.rows.1 && bottom > self.rows.0
        })
    }
}

impl FrameIndex {
    #[expect(
        clippy::unused_self,
        reason = "compile-time counterpart of the indexed scratch bound"
    )]
    pub const fn coverage_capacity(&self, _windowed: bool) -> Option<usize> {
        None
    }

    pub const fn rebuild(&mut self, _items: &[Item], height: usize) {
        self.height = height;
    }

    pub fn items<'a>(&self, band: usize, items: &'a [Item]) -> Selection<'a> {
        self.items_after(band, items, 0)
    }

    pub fn items_after<'a>(&self, band: usize, items: &'a [Item], start: usize) -> Selection<'a> {
        Selection {
            items,
            range: start..items.len(),
            rows: (band * BAND_H, ((band + 1) * BAND_H).min(self.height)),
            height: self.height,
        }
    }

    pub fn query_items(
        &self,
        range: Range<usize>,
        rows: (usize, usize),
        out: &mut SelectionBuffer,
    ) {
        out.range = range;
        // Match the indexed strategy's band-rounded window, including structural scopes.
        out.rows = if rows.0 >= rows.1 {
            (0, 0)
        } else {
            (
                rows.0 / BAND_H * BAND_H,
                (rows.1.div_ceil(BAND_H) * BAND_H).min(self.height),
            )
        };
    }

    pub fn selected<'a>(&self, buffer: &SelectionBuffer, items: &'a [Item]) -> Selection<'a> {
        Selection {
            items,
            range: buffer.range.clone(),
            rows: buffer.rows,
            height: self.height,
        }
    }

    #[expect(
        clippy::unused_self,
        reason = "same accounting interface as the indexed strategy"
    )]
    pub const fn heap_bytes(&self) -> u64 {
        0
    }
}
