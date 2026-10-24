//! Retained output validity, committed only after successful rasterization.

use std::ops::Range;

use cherenkov::LayerId;
use rustc_hash::FxHashMap;

use super::{bins, lower::Item, raster::BAND_H};

pub struct Layer {
    pub id: LayerId,
    pub revision: u64,
    pub items: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Footprint {
    revision: u64,
    rows: (usize, usize),
}

#[derive(Default)]
pub struct Damage {
    pub layers: Vec<Layer>,
    previous: FxHashMap<LayerId, Footprint>,
    prepared: FxHashMap<LayerId, Footprint>,
    stack: Vec<(Range<usize>, (usize, usize))>,
    bands: Vec<bool>,
    baseline: Option<([u32; 4], u64)>,
    pending: ([u32; 4], u64),
}

impl Damage {
    pub const fn forget(&mut self) {
        self.baseline = None;
    }

    pub fn heap_bytes(&self) -> u64 {
        (self.layers.capacity() * size_of::<Layer>()
            + (self.previous.capacity() + self.prepared.capacity())
                * size_of::<(LayerId, Footprint)>()
            + self.stack.capacity() * size_of::<(Range<usize>, (usize, usize))>()
            + self.bands.capacity() * size_of::<bool>()) as u64
    }

    pub fn prepare(
        &mut self,
        items: &[Item],
        h: usize,
        clear: [f32; 4],
        images: u64,
        animated: bool,
    ) {
        self.pending = (clear.map(f32::to_bits), images);
        let full = self.baseline != Some(self.pending) || animated;
        self.bands.clear();
        self.bands.resize(h.div_ceil(BAND_H), full);
        self.prepared.clear();
        self.stack.clear();
        // Layers arrive in postorder. Consume child aggregates and only scan
        // the gaps between them: every item is visited once, regardless of
        // the tree's depth.
        for layer in &self.layers {
            let mut rows = (h, 0);
            let mut end = layer.items.end;
            while self
                .stack
                .last()
                .is_some_and(|(range, _)| range.start >= layer.items.start)
            {
                let (child, child_rows) = self.stack.pop().expect("child aggregate");
                extend_items(&mut rows, &items[child.end..end], h);
                union(&mut rows, child_rows);
                end = child.start;
            }
            extend_items(&mut rows, &items[layer.items.start..end], h);
            self.stack.push((layer.items.clone(), rows));
            self.prepared.insert(
                layer.id,
                Footprint {
                    revision: layer.revision,
                    rows,
                },
            );
        }
        if !full {
            for (id, now) in &self.prepared {
                let old = self.previous.get(id);
                if old != Some(now) {
                    mark(&mut self.bands, now.rows);
                    if let Some(old) = old {
                        mark(&mut self.bands, old.rows);
                    }
                }
            }
            for (id, old) in &self.previous {
                if !self.prepared.contains_key(id) {
                    mark(&mut self.bands, old.rows);
                }
            }
            // These CPU kernels currently evaluate the entire filter window
            // or capture domain. Until their dependency windows are narrowed,
            // any changed input requires that domain to be evaluated again.
            if self.bands.iter().any(|dirty| *dirty)
                && items
                    .iter()
                    .any(|item| matches!(item, Item::PushFilter { .. } | Item::Capture(_)))
            {
                self.bands.fill(true);
            }
        }
    }

    pub fn bands(&self) -> &[bool] {
        &self.bands
    }

    pub const fn commit(&mut self) {
        std::mem::swap(&mut self.previous, &mut self.prepared);
        self.baseline = Some(self.pending);
    }
}

fn union(rows: &mut (usize, usize), other: (usize, usize)) {
    if other.0 < other.1 {
        rows.0 = rows.0.min(other.0);
        rows.1 = rows.1.max(other.1);
    }
}

fn extend_items(rows: &mut (usize, usize), items: &[Item], h: usize) {
    for item in items {
        // Normal isolation only combines its child pixels. Destructive
        // blending and filters can write transparent source pixels too.
        match item {
            Item::PushIsolate { .. } => {}
            Item::PopIsolate { blend, .. } if !super::blend::is_destructive(blend.0) => {}
            item => union(rows, bins::item_rows(item, h)),
        }
    }
}

fn mark(bands: &mut [bool], (start, end): (usize, usize)) {
    if start < end {
        bands[start / BAND_H..end.div_ceil(BAND_H)].fill(true);
    }
}
