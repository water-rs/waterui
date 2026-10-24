//! Command footprints survive source replacement and geometry-cache retirement.
//! Only submission promotes prepared footprints to the previous-frame baseline.

use std::{
    hash::{Hash, Hasher},
    ops::Range,
};

use cherenkov::{
    LayerId,
    damage::Region,
    kurbo::{Affine, Rect},
};
use rustc_hash::{FxHashMap, FxHasher};

use super::{
    instance::{FLAG_HAS_CLIP, Instance, KIND_REGION},
    lower::{Frame, Target},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub root: Target,
    pub layer: LayerId,
    pub op: usize,
}

pub struct Command {
    pub key: Key,
    pub instances: Range<usize>,
    pub revision: u64,
    pub mask_epoch: Option<u64>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Footprint {
    order: usize,
    bounds: [u32; 4],
    signature: u64,
}

#[derive(Default)]
pub struct Commands {
    previous: FxHashMap<(Key, usize), Footprint>,
    prepared: FxHashMap<(Key, usize), Footprint>,
    previous_targets: Vec<Target>,
    targets: Vec<Target>,
    spans: Vec<(Range<usize>, usize)>,
    regions: Vec<Region>,
    last_order: Vec<usize>,
    pending: bool,
}

impl Commands {
    pub fn bytes(&self) -> u64 {
        ((self.previous.capacity() + self.prepared.capacity())
            * size_of::<((Key, usize), Footprint)>()
            + (self.previous_targets.capacity() + self.targets.capacity()) * size_of::<Target>()
            + self.spans.capacity() * size_of::<(Range<usize>, usize)>()
            + self.regions.capacity() * size_of::<Region>()
            + self.last_order.capacity() * size_of::<usize>()) as u64
            + self
                .regions
                .iter()
                .map(|region| region.heap_bytes().0)
                .sum::<u64>()
    }

    pub fn pass(&self, index: usize) -> &Region {
        &self.regions[index]
    }

    pub fn pixels(&self) -> u64 {
        self.regions.iter().map(Region::pixel_area).sum()
    }

    pub fn update(&mut self, frame: &Frame) {
        self.begin(frame);
        self.collect(frame);
        if self.targets != self.previous_targets {
            for (region, pass) in self.regions.iter_mut().zip(&frame.passes) {
                let [x, y, width, height] = pass.region;
                region.add(x, y, width, height);
            }
            return;
        }
        for (key, now) in &self.prepared {
            let before = self.previous.get(key);
            if before.is_none_or(|old| old.bounds != now.bounds || old.signature != now.signature) {
                add(&mut self.regions[key.1], now.bounds);
                if let Some(before) = before {
                    add(&mut self.regions[key.1], before.bounds);
                }
            }
        }
        for (key, before) in &self.previous {
            if !self.prepared.contains_key(key) {
                add(&mut self.regions[key.1], before.bounds);
            }
        }
    }

    fn begin(&mut self, frame: &Frame) {
        self.pending = true;
        self.prepared.clear();
        self.last_order.clear();
        self.last_order.resize(frame.passes.len(), 0);
        self.targets.clear();
        self.targets
            .extend(frame.passes.iter().map(|pass| pass.target));
        self.spans.clear();
        for (pass, data) in frame.passes.iter().enumerate() {
            self.spans.extend(data.ranges.iter().map(|range| {
                (
                    range.instances.start as usize..range.instances.end as usize,
                    pass,
                )
            }));
        }
        self.spans.sort_unstable_by_key(|(span, _)| span.start);
        self.regions.truncate(frame.passes.len());
        for (index, pass) in frame.passes.iter().enumerate() {
            let [x, y, width, height] = pass.region;
            let extent = (x + width, y + height);
            if let Some(region) = self.regions.get_mut(index) {
                if region.extent() == extent {
                    region.clear();
                } else {
                    *region = Region::new(extent);
                }
            } else {
                self.regions.push(Region::new(extent));
            }
        }
    }

    fn collect(&mut self, frame: &Frame) {
        for (order, command) in frame.commands.iter().enumerate() {
            let start = self
                .spans
                .partition_point(|(span, _)| span.end <= command.instances.start);
            for (span, pass) in self.spans[start..]
                .iter()
                .take_while(|(span, _)| span.start < command.instances.end)
            {
                let instances = &frame.instances
                    [span.start.max(command.instances.start)..span.end.min(command.instances.end)];
                if instances.is_empty() {
                    continue;
                }
                let extent = self.regions[*pass].extent();
                let footprint = Footprint {
                    order,
                    bounds: super::lower::union_region(
                        instances
                            .iter()
                            .map(clipped_bounds)
                            .filter(|bounds| bounds[0] < bounds[2] && bounds[1] < bounds[3]),
                        extent.0,
                        extent.1,
                    ),
                    signature: signature(command, &instances[0]),
                };
                if let Some(before) = self.previous.get(&(command.key, *pass)) {
                    if before.order + 1 < self.last_order[*pass] {
                        // Reordering changes overlap even when each command's
                        // own pixels are unchanged. Insertions/removals alone
                        // preserve the relative order of surviving commands.
                        self.regions[*pass].mark_full();
                    }
                    self.last_order[*pass] = self.last_order[*pass].max(before.order + 1);
                }
                self.prepared
                    .entry((command.key, *pass))
                    .and_modify(|old| {
                        old.bounds = cover(old.bounds, footprint.bounds);
                        // One leaf can split across shader/raster ranges. Fold all
                        // templates in their emission order, not just the first.
                        old.signature = old.signature.rotate_left(7) ^ footprint.signature;
                    })
                    .or_insert(footprint);
            }
        }
    }

    pub fn submitted(&mut self) {
        if !std::mem::take(&mut self.pending) {
            return;
        }
        std::mem::swap(&mut self.previous, &mut self.prepared);
        std::mem::swap(&mut self.previous_targets, &mut self.targets);
    }
}

fn add(region: &mut Region, [x, y, width, height]: [u32; 4]) {
    region.add(x, y, width, height);
}

fn cover(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    if a[2] == 0 || a[3] == 0 {
        return b;
    }
    if b[2] == 0 || b[3] == 0 {
        return a;
    }
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    [
        x,
        y,
        (a[0] + a[2]).max(b[0] + b[2]) - x,
        (a[1] + a[3]).max(b[1] + b[3]) - y,
    ]
}

fn signature(command: &Command, instance: &Instance) -> u64 {
    let mut hash = FxHasher::default();
    command.revision.hash(&mut hash);
    command.mask_epoch.hash(&mut hash);
    let mut template = *instance;
    // Frame-local gradient offsets are allocation addresses, not paint.
    // Paint and geometry changes advance the retained emission revision.
    if template.meta[0] != KIND_REGION {
        template.meta[2] = 0;
    }
    hash.write(bytemuck::bytes_of(&template));
    hash.finish()
}

#[expect(
    clippy::cast_possible_truncation,
    reason = "GPU bounds are f32; clip math uses f64 before returning to that representation"
)]
fn clipped_bounds(instance: &Instance) -> [f32; 4] {
    let bounds = super::lower::device_bbox(instance);
    if instance.meta[3] & (FLAG_HAS_CLIP << 24) == 0 {
        return bounds;
    }
    let [xx, yx, xy, yy, tx, ty, _, _] = instance.clip_inv.map(f64::from);
    let [half_x, half_y] = instance.clip.half.map(f64::from);
    let clip = Affine::new([xx, yx, xy, yy, tx, ty])
        .inverse()
        .transform_rect_bbox(Rect::new(-half_x, -half_y, half_x, half_y));
    [
        bounds[0].max(clip.x0 as f32),
        bounds[1].max(clip.y0 as f32),
        bounds[2].min(clip.x1 as f32),
        bounds[3].min(clip.y1 as f32),
    ]
}

#[cfg(test)]
#[path = "damage_tests.rs"]
mod tests;
