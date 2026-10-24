//! Retained output validity, committed only after successful rasterization.

use std::ops::Range;

use cherenkov::{LayerId, damage::Region};
use rustc_hash::FxHashMap;

use super::{
    bins,
    lower::{CaptureItem, Item},
    raster::BAND_H,
};

struct Dependency {
    input: Range<usize>,
    output: (usize, usize),
}

impl Dependency {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "capture grid rows are converted outward and clamped to surface rows"
    )]
    fn capture(item: &CaptureItem, h: usize) -> Self {
        let scale = f64::from(item.scale.get());
        let start = (f64::from(item.region.y0) / scale)
            .floor()
            .clamp(0.0, h as f64) as usize;
        let end = (f64::from(item.region.y1) / scale)
            .ceil()
            .clamp(0.0, h as f64) as usize;
        Self {
            input: start / BAND_H..end.div_ceil(BAND_H),
            output: (
                (item.union.y0.max(0) as usize).min(h),
                (item.union.y1.max(0) as usize).min(h),
            ),
        }
    }
}

pub struct Layer {
    pub id: LayerId,
    pub revision: u64,
    pub items: Range<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Footprint {
    revision: u64,
    rows: (usize, usize),
    generation: u64,
}

pub struct Damage {
    pub layers: Vec<Layer>,
    previous: FxHashMap<LayerId, Footprint>,
    generation: u64,
    stack: Vec<(Range<usize>, (usize, usize))>,
    bands: Region,
    filter_aprons: Vec<usize>,
    captures: Vec<Dependency>,
    baseline: Option<([u32; 4], u64)>,
    pending: ([u32; 4], u64),
}

impl Default for Damage {
    fn default() -> Self {
        Self {
            layers: Vec::new(),
            previous: FxHashMap::default(),
            generation: 0,
            stack: Vec::new(),
            bands: Region::new((1, 0)),
            filter_aprons: Vec::new(),
            captures: Vec::new(),
            baseline: None,
            pending: ([0; 4], 0),
        }
    }
}

impl Damage {
    pub const fn forget(&mut self) {
        self.baseline = None;
    }

    pub fn heap_bytes(&self) -> u64 {
        (self.layers.capacity() * size_of::<Layer>()
            + self.previous.capacity() * size_of::<(LayerId, Footprint)>()
            + self.stack.capacity() * size_of::<(Range<usize>, (usize, usize))>()) as u64
            + self.bands.heap_bytes().0
            + (self.filter_aprons.capacity() * size_of::<usize>()) as u64
            + (self.captures.capacity() * size_of::<Dependency>()) as u64
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
        // Footprints update in place. Until rasterization commits, neither
        // this metadata nor a partially written output is a valid baseline.
        let full = self.baseline.take() != Some(self.pending) || animated;
        let extent = (
            1,
            u32::try_from(h.div_ceil(BAND_H)).expect("surface band count fits u32"),
        );
        if self.bands.extent() != extent {
            self.bands = Region::new(extent);
        }
        self.bands.clear();
        if full {
            self.bands.mark_full();
        }
        self.generation += 1;
        self.stack.clear();
        let apron = self.dependencies(items, h);
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
            let now = Footprint {
                revision: layer.revision,
                rows,
                generation: self.generation,
            };
            let old = self.previous.insert(layer.id, now);
            if !full && old.is_none_or(|old| old.revision != now.revision || old.rows != now.rows) {
                mark(&mut self.bands, now.rows, apron, h);
                if let Some(old) = old {
                    mark(&mut self.bands, old.rows, apron, h);
                }
            }
        }
        self.previous.retain(|_, old| {
            let retained = old.generation == self.generation;
            if !retained && !full {
                mark(&mut self.bands, old.rows, apron, h);
            }
            retained
        });
        if !full {
            // A capture can read an earlier capture's output. Propagate to
            // a fixed point; each iteration only adds selected bands.
            loop {
                let mut changed = false;
                for capture in &self.captures {
                    if self.bands().rectangles().iter().any(|rect| {
                        (rect.y0() as usize) < capture.input.end
                            && capture.input.start < rect.y1() as usize
                    }) {
                        changed |= mark(&mut self.bands, capture.output, apron, h);
                    }
                }
                if !changed {
                    break;
                }
            }
        }
    }

    fn dependencies(&mut self, items: &[Item], h: usize) -> usize {
        self.filter_aprons.clear();
        self.captures.clear();
        let (mut active, mut maximum) = (0, 0);
        for item in items {
            match item {
                Item::PushFilter { apron, .. } => {
                    self.filter_aprons.push(*apron);
                    active += apron;
                    maximum = maximum.max(active);
                }
                Item::PopFilter { .. } => {
                    active -= self.filter_aprons.pop().expect("paired filter scope");
                }
                Item::Capture(capture) => self.captures.push(Dependency::capture(capture, h)),
                _ => {}
            }
        }
        maximum.min(h)
    }

    pub const fn bands(&self) -> &Region {
        &self.bands
    }

    pub const fn commit(&mut self) {
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

fn mark(bands: &mut Region, (start, end): (usize, usize), apron: usize, h: usize) -> bool {
    if start >= end || bands.is_full() {
        return false;
    }
    let start = start.saturating_sub(apron) / BAND_H;
    let end = (end + apron).min(h).div_ceil(BAND_H);
    let before = bands.pixel_area();
    if start == 0 && end == h.div_ceil(BAND_H) {
        bands.mark_full();
    } else {
        bands.add(
            0,
            u32::try_from(start).expect("band start fits u32"),
            1,
            u32::try_from(end - start).expect("band count fits u32"),
        );
    }
    bands.pixel_area() != before
}
