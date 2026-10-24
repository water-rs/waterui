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
    lower::{DrawRange, Frame, Target},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    pub root: Target,
    pub layer: LayerId,
    pub op: usize,
}

/// Consecutive leaves occupy one contiguous instance span; scope-generated
/// instances lie between sources, never between commands of one source.
pub struct CommandSource {
    pub commands: Range<usize>,
    pub instances: Range<usize>,
    pub first_op: usize,
    pub root: Target,
    pub layer: LayerId,
    pub mask_epoch: Option<u64>,
}

pub struct Command {
    /// Starts at the source start or the preceding command end.
    pub instances_end: usize,
    pub revision: u64,
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
    previous_states: Vec<u64>,
    states: Vec<u64>,
    targets: Vec<Target>,
    spans: Vec<(Range<usize>, usize, u64)>,
    regions: Vec<Region>,
    last_order: Vec<usize>,
    pending: bool,
}

impl Commands {
    pub fn forget(&mut self) {
        self.previous.clear();
        self.previous_targets.clear();
        self.previous_states.clear();
        self.pending = false;
    }

    pub fn bytes(&self) -> u64 {
        ((self.previous.capacity() + self.prepared.capacity())
            * size_of::<((Key, usize), Footprint)>()
            + (self.previous_targets.capacity() + self.targets.capacity()) * size_of::<Target>()
            + (self.previous_states.capacity() + self.states.capacity()) * size_of::<u64>()
            + self.spans.capacity() * size_of::<(Range<usize>, usize, u64)>()
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

    #[cfg(test)]
    pub fn update(&mut self, frame: &Frame) {
        self.update_resources(frame, |_| 0);
    }

    pub fn update_resources(&mut self, frame: &Frame, version: impl Fn(&DrawRange) -> u64) {
        self.begin(frame, &version);
        self.collect(frame);
        if self.targets != self.previous_targets {
            for (region, pass) in self.regions.iter_mut().zip(&frame.passes) {
                let [x, y, width, height] = pass.region;
                region.add(x, y, width, height);
            }
            return;
        }
        for (index, (before, now)) in self.previous_states.iter().zip(&self.states).enumerate() {
            if before != now {
                add(&mut self.regions[index], frame.passes[index].region);
            }
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

    fn begin(&mut self, frame: &Frame, version: &impl Fn(&DrawRange) -> u64) {
        self.pending = true;
        self.prepared.clear();
        self.last_order.clear();
        self.last_order.resize(frame.passes.len(), 0);
        self.targets.clear();
        self.targets
            .extend(frame.passes.iter().map(|pass| pass.target));
        self.states.clear();
        self.states
            .extend((0..frame.passes.len()).map(|index| pass_state(frame, index, version)));
        self.spans.clear();
        for (pass, data) in frame.passes.iter().enumerate() {
            self.spans.extend(data.ranges.iter().map(|range| {
                (
                    range.instances.start as usize..range.instances.end as usize,
                    pass,
                    range_state(range, version(range)),
                )
            }));
        }
        self.spans.sort_unstable_by_key(|(span, _, _)| span.start);
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
        for source in &frame.command_sources {
            let mut first_instance = source.instances.start;
            for (offset, command) in frame.commands[source.commands.clone()].iter().enumerate() {
                let command_instances = first_instance..command.instances_end;
                first_instance = command.instances_end;
                let order = source.commands.start + offset;
                let key = Key {
                    root: source.root,
                    layer: source.layer,
                    op: source.first_op + offset,
                };
                let start = self
                    .spans
                    .partition_point(|(span, _, _)| span.end <= command_instances.start);
                for (span, pass, binding) in self.spans[start..]
                    .iter()
                    .take_while(|(span, _, _)| span.start < command_instances.end)
                {
                    let instances = &frame.instances[span.start.max(command_instances.start)
                        ..span.end.min(command_instances.end)];
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
                        signature: signature(command, source, &instances[0], *binding),
                    };
                    if let Some(before) = self.previous.get(&(key, *pass)) {
                        if before.order + 1 < self.last_order[*pass] {
                            // Reordering changes overlap even when each command's
                            // own pixels are unchanged. Insertions/removals alone
                            // preserve the relative order of surviving commands.
                            self.regions[*pass].mark_full();
                        }
                        self.last_order[*pass] = self.last_order[*pass].max(before.order + 1);
                    }
                    self.prepared
                        .entry((key, *pass))
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
    }

    pub fn invalidate_pass(&mut self, frame: &Frame, index: usize) {
        add(&mut self.regions[index], frame.passes[index].region);
    }

    pub fn invalidate_range(&mut self, frame: &Frame, index: usize, range: &DrawRange) {
        let extent = self.regions[index].extent();
        let bounds = super::lower::union_region(
            frame.instances[range.instances.start as usize..range.instances.end as usize]
                .iter()
                .map(clipped_bounds),
            extent.0,
            extent.1,
        );
        add(&mut self.regions[index], bounds);
    }

    pub fn submitted(&mut self) {
        if !std::mem::take(&mut self.pending) {
            return;
        }
        std::mem::swap(&mut self.previous, &mut self.prepared);
        std::mem::swap(&mut self.previous_targets, &mut self.targets);
        std::mem::swap(&mut self.previous_states, &mut self.states);
    }
}

/// State not owned by a leaf emission: attachment operations and synthetic
/// composites. Instance buffer offsets are not semantic state.
fn pass_state(frame: &Frame, index: usize, version: &impl Fn(&DrawRange) -> u64) -> u64 {
    let pass = &frame.passes[index];
    let mut hash = FxHasher::default();
    pass.clear
        .map(|color| color.map(f32::to_bits))
        .hash(&mut hash);
    pass.region.hash(&mut hash);
    (pass.space == cherenkov::BlendSpace::Linear).hash(&mut hash);
    pass.backdrop_copy.hash(&mut hash);
    pass.capture
        .map(|capture| {
            (
                capture.group,
                capture.region,
                capture.copy_from,
                capture.levels,
                capture
                    .resolve
                    .map(|resolve| (resolve.scale.to_bits(), resolve.device, resolve.extent)),
            )
        })
        .hash(&mut hash);
    for (_, key) in frame.filters.iter().filter(|(owner, _)| *owner == index) {
        key.hash(&mut hash);
    }
    for (_, parameters) in frame.shadows.iter().filter(|(owner, _)| *owner == index) {
        parameters
            .transform
            .as_coeffs()
            .map(f64::to_bits)
            .hash(&mut hash);
        parameters.sigma.to_bits().hash(&mut hash);
        parameters.spread.to_bits().hash(&mut hash);
    }
    for (_, levels) in frame.reduces.iter().filter(|(owner, _)| *owner == index) {
        levels.hash(&mut hash);
    }
    for range in &pass.ranges {
        let mut cursor = range.instances.start as usize;
        let end = range.instances.end as usize;
        let first = frame
            .command_sources
            .partition_point(|command| command.instances.end <= cursor);
        for command in frame.command_sources[first..]
            .iter()
            .take_while(|command| command.instances.start < end)
        {
            let synthetic_end = command.instances.start.min(end);
            if cursor < synthetic_end {
                range_state(range, version(range)).hash(&mut hash);
                synthetic_state(frame, cursor..synthetic_end, &mut hash);
            }
            cursor = cursor.max(command.instances.end).min(end);
        }
        if cursor < end {
            range_state(range, version(range)).hash(&mut hash);
            synthetic_state(frame, cursor..end, &mut hash);
        }
    }
    hash.finish()
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "union headers encode validated bounded integer member counts"
)]
fn synthetic_state(frame: &Frame, instances: Range<usize>, hash: &mut FxHasher) {
    use super::instance::{FLAG_UNION, PAINT_BACKDROP, PAINT_PROJECTIVE};
    for instance in &frame.instances[instances] {
        let mut template = *instance;
        let count = match instance.meta[1] {
            PAINT_BACKDROP => ((instance.meta[3] >> 8) & 0xff) as usize,
            PAINT_PROJECTIVE => 3,
            _ => 0,
        };
        if count != 0 {
            let first = instance.meta[2] as usize;
            hash.write(bytemuck::cast_slice(&frame.stops[first..first + count]));
            template.meta[2] = 0;
        }
        if instance.meta[3] & (FLAG_UNION << 24) != 0 {
            let first = instance.uv[0].to_bits() as usize;
            let count = 1 + 2 * frame.stops[first].color[1] as usize;
            hash.write(bytemuck::cast_slice(&frame.stops[first..first + count]));
            template.uv[0] = 0.0;
        }
        hash.write(bytemuck::bytes_of(&template));
    }
}

fn range_state(range: &DrawRange, version: u64) -> u64 {
    let mut hash = FxHasher::default();
    version.hash(&mut hash);
    range.source.hash(&mut hash);
    range.image.hash(&mut hash);
    range.mask.hash(&mut hash);
    range.pipeline.hash(&mut hash);
    range.variant.hash(&mut hash);
    hash.finish()
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

fn signature(command: &Command, source: &CommandSource, instance: &Instance, binding: u64) -> u64 {
    let mut hash = FxHasher::default();
    binding.hash(&mut hash);
    command.revision.hash(&mut hash);
    source.mask_epoch.hash(&mut hash);
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
