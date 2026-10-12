//! Image lifetimes derived from the completed canonical frame.
//!
//! A scratch depth is an allocation address, not an image identity. Every
//! clear starts a new generation; reads name that generation before any
//! physical attachment is assigned.
//!
//! Nothing here is platform-specific (issue #2549). A backend hands the
//! planner the [`Limits`] its tile executor works within; a target with
//! no executor has no limits, so an [`ExecutionPlan`] is never built
//! there and every logical image is materialized for
//! [`Materialization::NoExecutor`].

// A target with no executor never calls the planner, so every item here
// is unreachable there. Both executors — Apple's (#2549) and Vulkan's
// (#2191) — use all of it, so only a target with neither has dead code
// here.
#![cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "no tile executor on this target, so nothing plans (#2549)"
    )
)]

use std::ops::Range;

use super::super::composition::Materialization;
use super::super::instance::PAINT_TEXTURE;
use super::super::lower::{DrawRange, Frame, ImageSource, PipelineKind, Source, Target};

/// How a tile executor orders two accesses to one pixel of one
/// attachment slot within a single pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ordering {
    /// Fragments covering the same pixel are ordered against each other,
    /// so a draw reads exactly what the epoch's earlier draws wrote into
    /// the slot — Apple's `raster_order_group(0)`.
    PerPixel,
    /// Accesses are ordered only at pass granularity: a draw that reads
    /// a slot an earlier draw of the same pass wrote is undefined, so no
    /// epoch can carry a destination or slot read.
    PerPass,
}

/// What a backend's tile executor can hold, as the planner must respect
/// it: how many attachment slots it binds at once (slot zero, the stored
/// output, included), the pixel format every slot carries, and the
/// ordering it guarantees within one pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Attachment slots the executor binds at once, slot zero included.
    /// Fewer than two slots cannot hold an intermediate beside the
    /// output, so no epoch forms.
    pub slots: u8,
    /// The format every attachment slot carries. An image stored in any
    /// other format cannot enter an epoch.
    pub format: wgpu::TextureFormat,
    /// The ordering the executor guarantees within one pass.
    pub ordering: Ordering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogicalImageId(pub usize);

#[derive(Clone, Debug)]
pub struct LogicalImage {
    pub target: Target,
    pub generation: usize,
    pub region: [u32; 4],
    pub space: cherenkov::BlendSpace,
    pub format: wgpu::TextureFormat,
    pub producer: Option<usize>,
    pub last_consumer: usize,
    pub storage: Storage,
}

/// Where a logical image's pixels live for the frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Storage {
    /// The image never reaches device memory: it lives in a tile epoch's
    /// attachment slot and is read back through attachment fetch.
    OnChip,
    /// The image is written to a device-memory texture, for this reason.
    Materialized(Materialization),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadKind {
    CorrespondingPixel,
    FrozenSnapshot,
    Neighborhood,
    Opaque,
}

#[derive(Clone, Debug)]
pub struct Read {
    pub image: LogicalImageId,
    pub kind: ReadKind,
}

#[derive(Clone, Debug)]
pub struct TileOp {
    pub pass: usize,
    pub target: u8,
    pub sources: Vec<Option<u8>>,
    pub begin: bool,
    /// Where a `begin` whose slot still holds a dead image has to clear:
    /// every region this epoch writes that image over, and every region
    /// it reads the image from, so the slot reads transparent everywhere
    /// the image's own clear would have. Only `begin` consults it.
    pub clear_region: [u32; 4],
    pub backdrop: Option<ReadKind>,
}

/// Slot zero is the persistent output. Other slots have no device-memory
/// backing and are cleared at each logical image's original begin point.
///
/// One epoch is one native pass. [`ExecutionPlan::partition`] coalesces
/// adjacent intervals that store into the same logical image into a
/// single epoch, so an epoch's ops can cover several intervals and the
/// output stays in tile memory across all of them.
#[derive(Clone, Debug)]
pub struct TileEpoch {
    pub passes: Range<usize>,
    pub output: LogicalImageId,
    pub region: [u32; 4],
    pub slots: u8,
    pub ops: Vec<TileOp>,
}

#[derive(Clone, Debug)]
pub enum Segment {
    Portable(Range<usize>),
    NativeEpoch(TileEpoch),
}

#[derive(Default)]
pub struct ExecutionPlan {
    pub images: Vec<LogicalImage>,
    pub targets: Vec<LogicalImageId>,
    pub reads: Vec<Vec<Option<Read>>>,
    pub segments: Vec<Segment>,
    native: Vec<Option<usize>>,
    /// Whether a backend planned this frame at all. A target whose
    /// backend has no tile executor supplies no [`Limits`], so its plan
    /// stays in this default state: no epoch, and every image
    /// materialized for [`Self::unplanned`]'s reason.
    planned: bool,
    /// Why no plan was built, while `planned` is false.
    ///
    /// `None` is the default state [`Materialization::NoExecutor`]
    /// names: the target has no tile executor and nothing asked. A
    /// backend that *has* an executor but could not install it —
    /// because the device lacks what it needs, or the engine is
    /// configured with a format it cannot bind — records that reason
    /// here through [`ExecutionPlan::set_unplanned`], so the
    /// attribution says which (#2191).
    unplanned: Option<Materialization>,
}

impl ExecutionPlan {
    pub fn bytes(&self) -> u64 {
        (self.images.capacity() * std::mem::size_of::<LogicalImage>()
            + self.native.capacity() * std::mem::size_of::<Option<usize>>()
            + self.targets.capacity() * std::mem::size_of::<LogicalImageId>()
            + self.reads.capacity() * std::mem::size_of::<Vec<Option<Read>>>()
            + self
                .reads
                .iter()
                .map(|reads| reads.capacity() * std::mem::size_of::<Option<Read>>())
                .sum::<usize>()
            + self.segments.capacity() * std::mem::size_of::<Segment>()
            + self
                .segments
                .iter()
                .map(|segment| match segment {
                    Segment::Portable(_) => 0,
                    Segment::NativeEpoch(epoch) => {
                        epoch.ops.capacity() * std::mem::size_of::<TileOp>()
                            + epoch
                                .ops
                                .iter()
                                .map(|op| op.sources.capacity() * std::mem::size_of::<Option<u8>>())
                                .sum::<usize>()
                    }
                })
                .sum::<usize>()) as u64
    }

    /// The plan for `frame`, where `scratch` is the format the frame's
    /// isolation scratches are stored in and `limits` is what the
    /// backend's tile executor can hold.
    pub fn build(frame: &Frame, scratch: wgpu::TextureFormat, limits: Limits) -> Self {
        let mut plan = Self {
            planned: true,
            ..Self::default()
        };
        let mut current: Vec<(Target, LogicalImageId)> = Vec::new();
        for (index, pass) in frame.passes.iter().enumerate() {
            let previous = current
                .iter()
                .position(|(target, _)| *target == pass.target);
            let image = if let Some(previous) = previous.filter(|_| pass.clear.is_none()) {
                current[previous].1
            } else {
                let generation = previous.map_or(0, |p| plan.images[current[p].1.0].generation + 1);
                let id = LogicalImageId(plan.images.len());
                plan.images.push(LogicalImage {
                    target: pass.target,
                    generation,
                    region: pass.region,
                    space: pass.space,
                    format: if matches!(pass.target, Target::Scratch(_)) {
                        scratch
                    } else {
                        wgpu::TextureFormat::Rgba16Float
                    },
                    producer: Some(index),
                    last_consumer: index,
                    storage: Storage::Materialized(initial(pass.target)),
                });
                if let Some(p) = previous {
                    current[p].1 = id;
                } else {
                    current.push((pass.target, id));
                }
                id
            };
            plan.images[image.0].last_consumer = index;
            plan.targets.push(image);
            let reads = pass
                .ranges
                .iter()
                .map(|range| {
                    let source = range.source?;
                    let target = source_target(source);
                    let id = if let Some((_, id)) = current.iter().find(|(t, _)| *t == target) {
                        *id
                    } else {
                        // Retained projected images can be sampled without a
                        // producer in this frame. They enter already materialized.
                        assert!(matches!(target, Target::Projected(_)));
                        let id = LogicalImageId(plan.images.len());
                        plan.images.push(LogicalImage {
                            target,
                            generation: 0,
                            region: [0; 4],
                            space: cherenkov::BlendSpace::Linear,
                            format: wgpu::TextureFormat::Rgba16Float,
                            producer: None,
                            last_consumer: index,
                            storage: Storage::Materialized(Materialization::Persistent),
                        });
                        current.push((target, id));
                        id
                    };
                    plan.images[id.0].last_consumer = index;
                    let kind = read_kind(frame, range);
                    Some(Read { image: id, kind })
                })
                .collect();
            plan.reads.push(reads);
            if let Some(capture) = pass.capture {
                let id = current
                    .iter()
                    .find(|(t, _)| *t == capture.copy_from)
                    .expect("capture source exists")
                    .1;
                plan.images[id.0].last_consumer = index;
            }
        }
        plan.partition(frame, limits);
        plan
    }

    pub fn update_placement(&mut self, frame: &Frame, limits: Limits) {
        for (index, pass) in frame.passes.iter().enumerate() {
            let image = &mut self.images[self.targets[index].0];
            image.region = pass.region;
            image.space = pass.space;
            image.storage = Storage::Materialized(initial(pass.target));
        }
        self.segments.clear();
        self.partition(frame, limits);
    }

    fn partition(&mut self, frame: &Frame, limits: Limits) {
        let plan = self;
        plan.native.clear();
        plan.native.resize(frame.passes.len(), None);
        let mut index = 0;
        while index < frame.passes.len() {
            // An epoch's draws read slots earlier draws of the same pass
            // wrote, so an executor that orders accesses only between
            // passes can carry none of it.
            let attempt = if limits.ordering == Ordering::PerPass {
                Err(Materialization::Ordering)
            } else if limits.slots >= 2 {
                plan.epoch(frame, index, limits)
            } else {
                Err(Materialization::SlotLimit)
            };
            match attempt {
                Ok(epoch) => {
                    for op in &epoch.ops {
                        let id = plan.targets[op.pass];
                        if id != epoch.output {
                            plan.images[id.0].storage = Storage::OnChip;
                        }
                    }
                    // An output the epoch itself produces is stored by this
                    // epoch and never visited as an interval of its own.
                    let output = &mut plan.images[epoch.output.0];
                    if output.producer.is_some_and(|p| epoch.passes.contains(&p)) {
                        output.storage =
                            Storage::Materialized(if matches!(output.target, Target::Scratch(_)) {
                                // Its consumers lie past the epoch's end.
                                Materialization::External
                            } else {
                                Materialization::Persistent
                            });
                    }
                    // Adjacent intervals that store into the same logical
                    // image are one native pass: nothing between them
                    // touches that image — a gap would have opened a
                    // portable segment, and a clear would have started a
                    // new image — so its pixels need never leave the
                    // executor's slot zero between them. The interval's own
                    // slots are free again at its end (an image living past
                    // it is `External`), which is what lets the next one
                    // reuse them under the same attachment binding.
                    let coalesce = match plan.segments.last() {
                        Some(Segment::NativeEpoch(last)) => {
                            let stores_the_same_image = last.output == epoch.output;
                            let adjacent = last.passes.end == epoch.passes.start;
                            stores_the_same_image && adjacent
                        }
                        _ => false,
                    };
                    let passes = epoch.passes.clone();
                    index = passes.end;
                    if coalesce {
                        let Some(Segment::NativeEpoch(last)) = plan.segments.last_mut() else {
                            unreachable!("the segment just matched a native epoch")
                        };
                        debug_assert_eq!(
                            last.region, epoch.region,
                            "one logical image has one region"
                        );
                        last.passes.end = epoch.passes.end;
                        last.slots = last.slots.max(epoch.slots);
                        last.ops.extend(epoch.ops);
                    } else {
                        plan.segments.push(Segment::NativeEpoch(epoch));
                    }
                    plan.native[passes].fill(Some(plan.segments.len() - 1));
                }
                Err(reason) => {
                    // The reason belongs to the attempt that opens the image;
                    // a pass continuing an earlier interval never revises it.
                    let image = &mut plan.images[plan.targets[index].0];
                    if image.producer == Some(index) {
                        image.storage = Storage::Materialized(reason);
                    }
                    if let Some(Segment::Portable(range)) = plan.segments.last_mut() {
                        range.end = index + 1;
                    } else {
                        plan.segments.push(Segment::Portable(index..index + 1));
                    }
                    index += 1;
                }
            }
        }
    }

    /// The interval an epoch rooted at `start` would cover: the image that
    /// pass produces, read until its last consumer, which stores the
    /// epoch's result. `Err` names why that interval does not exist.
    fn interval(&self, start: usize, limits: Limits) -> Result<usize, Materialization> {
        let first = &self.images[self.targets[start].0];
        if !matches!(first.target, Target::Scratch(_)) {
            return Err(Materialization::Persistent);
        }
        if first.producer != Some(start) {
            return Err(Materialization::NoInterval);
        }
        let end = first.last_consumer;
        if end == start {
            return Err(Materialization::NoInterval);
        }
        let output = self.targets[end];
        let destination = &self.images[output.0];
        if output == self.targets[start]
            || !matches!(destination.target, Target::Part(_) | Target::Scratch(_))
        {
            return Err(Materialization::OutputNotEligible);
        }
        if destination.format != limits.format {
            return Err(Materialization::Format);
        }
        Ok(end)
    }

    /// Whether pass `index` may run inside an epoch storing into
    /// `destination`, independently of the slots it would need: a
    /// filtered result, custom effect, retained image, external
    /// operation or snapshot with overlapping consumers cannot cross a
    /// tile epoch.
    fn admits(
        &self,
        frame: &Frame,
        index: usize,
        destination: &LogicalImage,
        limits: Limits,
    ) -> Result<(), Materialization> {
        let pass = &frame.passes[index];
        let image = &self.images[self.targets[index].0];
        if materialization_boundary(frame, index) {
            return Err(Materialization::UnsupportedOp);
        }
        if pass.capture.is_some() {
            return Err(Materialization::External);
        }
        if image.format != limits.format {
            return Err(Materialization::Format);
        }
        if image.space != pass.space {
            return Err(Materialization::BlendSpace);
        }
        if !contains(destination.region, image.region) {
            return Err(Materialization::Region);
        }
        if pass.ranges.iter().any(|range| {
            !matches!(
                range.pipeline,
                PipelineKind::SrcOver | PipelineKind::Replace
            ) || matches!(
                range.image,
                Some(ImageSource::Content(_) | ImageSource::Shader(_))
            )
        }) {
            return Err(Materialization::UnsupportedOp);
        }
        // A single canonical composite quad cannot observe its own earlier
        // fragments. Only that proof permits replacing a frozen snapshot
        // with the live destination; all other snapshots materialize.
        if pass.backdrop_copy.is_some()
            && (pass.ranges.len() != 1
                || pass.ranges[0].instances.len() != 1
                || self.reads[index][0]
                    .as_ref()
                    .is_none_or(|read| read.kind != ReadKind::CorrespondingPixel))
        {
            return Err(Materialization::FrozenSnapshot);
        }
        Ok(())
    }

    /// The epoch rooted at `start`, or why the image that pass produces is
    /// materialized instead.
    fn epoch(
        &self,
        frame: &Frame,
        start: usize,
        limits: Limits,
    ) -> Result<TileEpoch, Materialization> {
        let end = self.interval(start, limits)?;
        let output = self.targets[end];
        let destination = &self.images[output.0];
        let mut assigned = vec![(output, 0u8)];
        let mut slots = 1;
        let mut ops = Vec::new();
        // Where each image this interval writes has to read transparent: a
        // slot it reuses still holds the previous occupant's pixels, which
        // a draw covers only inside its own region. Every region the
        // interval writes the image over counts, and so does the image's
        // own region, which `corresponds` proves bounds every read of it.
        let mut cleared: Vec<(LogicalImageId, [u32; 4])> = Vec::new();
        for index in start..=end {
            let id = self.targets[index];
            let region = frame.passes[index].region;
            match cleared.iter_mut().find(|(i, _)| *i == id) {
                Some((_, grown)) => *grown = cover(*grown, region),
                None => cleared.push((id, cover(self.images[id.0].region, region))),
            }
        }
        for index in start..=end {
            let pass = &frame.passes[index];
            let id = self.targets[index];
            let image = &self.images[id.0];
            self.admits(frame, index, destination, limits)?;
            let begin = image.producer == Some(index);
            let target = if let Some((_, slot)) = assigned.iter().find(|(i, _)| *i == id) {
                *slot
            } else {
                if !begin {
                    // Produced before this epoch opened: the interval that
                    // owns the image decides its storage, not this one.
                    return Err(Materialization::External);
                }
                if !matches!(image.target, Target::Scratch(_)) {
                    return Err(Materialization::OutputNotEligible);
                }
                if image.last_consumer > end {
                    return Err(Materialization::External);
                }
                if pass.clear != Some([0.0; 4]) {
                    return Err(Materialization::UnsupportedOp);
                }
                let slot = (1..limits.slots)
                    .find(|slot| {
                        assigned
                            .iter()
                            .all(|(i, s)| s != slot || self.images[i.0].last_consumer < index)
                    })
                    .ok_or(Materialization::SlotLimit)?;
                slots = slots.max(slot + 1);
                assigned.push((id, slot));
                slot
            };
            let mut sources = Vec::with_capacity(pass.ranges.len());
            for (read, range) in self.reads[index].iter().zip(&pass.ranges) {
                let slot = match read {
                    None => None,
                    Some(read) => match read.kind {
                        ReadKind::CorrespondingPixel => {
                            // Attachment fetch and texture fetch must identify
                            // the same pixel, the source's virtual extent
                            // included; a source outside the epoch has no slot.
                            let (_, slot) = assigned
                                .iter()
                                .find(|(i, _)| *i == read.image)
                                .ok_or(Materialization::External)?;
                            if !corresponds(frame, range, self.images[read.image.0].region) {
                                return Err(Materialization::NeighborhoodRead);
                            }
                            Some(*slot)
                        }
                        ReadKind::Neighborhood => return Err(Materialization::NeighborhoodRead),
                        ReadKind::FrozenSnapshot => return Err(Materialization::FrozenSnapshot),
                        ReadKind::Opaque => return Err(Materialization::UnsupportedOp),
                    },
                };
                sources.push(slot);
            }
            ops.push(TileOp {
                pass: index,
                target,
                sources,
                begin,
                clear_region: cleared
                    .iter()
                    .find(|(i, _)| *i == id)
                    .expect("every written image was covered")
                    .1,
                backdrop: pass.backdrop_copy.map(|_| ReadKind::FrozenSnapshot),
            });
        }
        Ok(TileEpoch {
            passes: start..end + 1,
            output,
            region: destination.region,
            slots,
            ops,
        })
    }

    /// Whether a backend planned this frame. Only a planned frame has
    /// logical images to report on (issue #2547's attribution).
    pub const fn is_planned(&self) -> bool {
        self.planned
    }

    /// Records that this frame is not planned, and why.
    ///
    /// The backend calls this instead of [`Self::build`] when it has an
    /// executor it could not install, so [`Self::materialization`]
    /// names the real obstacle rather than the generic
    /// [`Materialization::NoExecutor`]. Any plan a previous frame built
    /// is discarded: an unplanned frame has no epochs.
    pub fn set_unplanned(&mut self, reason: Materialization) {
        if self.planned || self.unplanned != Some(reason) {
            *self = Self {
                unplanned: Some(reason),
                ..Self::default()
            };
        }
    }

    /// Why the image `pass` writes reaches device memory; `None` when an
    /// epoch keeps it on chip. The pass needs a backed scratch attachment
    /// exactly when this is `Some`.
    pub fn materialization(&self, pass: usize) -> Option<Materialization> {
        if !self.planned {
            return Some(self.unplanned.unwrap_or(Materialization::NoExecutor));
        }
        match self.images[self.targets[pass].0].storage {
            Storage::OnChip => None,
            Storage::Materialized(reason) => Some(reason),
        }
    }

    /// The frame's tile epochs, in pass order.
    pub fn epochs(&self) -> impl Iterator<Item = &TileEpoch> {
        self.segments.iter().filter_map(|segment| match segment {
            Segment::Portable(_) => None,
            Segment::NativeEpoch(epoch) => Some(epoch),
        })
    }

    /// The segment index of the epoch covering `pass`, if one does. An
    /// unplanned frame has no entry for any pass.
    fn native_segment(&self, pass: usize) -> Option<usize> {
        self.native.get(pass).copied().flatten()
    }

    pub fn epoch_at(&self, pass: usize) -> Option<&TileEpoch> {
        self.native_segment(pass)
            .and_then(|index| match &self.segments[index] {
                Segment::NativeEpoch(epoch) if epoch.passes.start == pass => Some(epoch),
                _ => None,
            })
    }

    pub fn contains_native_pass(&self, pass: usize) -> bool {
        self.native_segment(pass).is_some()
    }

    pub fn attachment_origin(&self, pass: usize) -> Option<[u32; 2]> {
        self.native_segment(pass).map(|index| {
            let Segment::NativeEpoch(epoch) = &self.segments[index] else {
                unreachable!("native pass index names its epoch")
            };
            [epoch.region[0], epoch.region[1]]
        })
    }
}

pub fn read_kind(frame: &Frame, range: &DrawRange) -> ReadKind {
    if range.source.is_none() || matches!(range.pipeline, PipelineKind::Effect { .. }) {
        ReadKind::Opaque
    } else if frame.instances[range.instances.start as usize..range.instances.end as usize]
        .iter()
        .all(|instance| instance.meta[1] == PAINT_TEXTURE)
    {
        ReadKind::CorrespondingPixel
    } else {
        ReadKind::Neighborhood
    }
}

/// The storage reason an image carries before the partition decides it:
/// a non-scratch target is persistent by construction, and a scratch has
/// no proven interval until an epoch attempt names one.
const fn initial(target: Target) -> Materialization {
    if matches!(target, Target::Scratch(_)) {
        Materialization::NoInterval
    } else {
        Materialization::Persistent
    }
}

pub fn materialization_boundary(frame: &Frame, index: usize) -> bool {
    frame.filters.iter().any(|(i, _)| *i == index)
        || frame.shadows.iter().any(|(i, _)| *i == index)
        || frame.mips.iter().any(|(i, _)| *i == index)
}

/// The smallest region containing both.
fn cover(a: [u32; 4], b: [u32; 4]) -> [u32; 4] {
    let x = a[0].min(b[0]);
    let y = a[1].min(b[1]);
    [
        x,
        y,
        (a[0] + a[2]).max(b[0] + b[2]) - x,
        (a[1] + a[3]).max(b[1] + b[3]) - y,
    ]
}

const fn contains(outer: [u32; 4], inner: [u32; 4]) -> bool {
    let [outer_x, outer_y, outer_width, outer_height] = outer;
    let [inner_x, inner_y, inner_width, inner_height] = inner;
    inner_x >= outer_x
        && inner_y >= outer_y
        && inner_x + inner_width <= outer_x + outer_width
        && inner_y + inner_height <= outer_y + outer_height
}

#[expect(
    clippy::cast_precision_loss,
    reason = "device pixel coordinates fit f32"
)]
fn corresponds(frame: &Frame, range: &DrawRange, region: [u32; 4]) -> bool {
    let [x, y, width, height] = region.map(|v| v as f32);
    frame.instances[range.instances.start as usize..range.instances.end as usize]
        .iter()
        .all(|instance| {
            instance.grad[..2] == [x, y]
                && instance.bounds[0] >= x
                && instance.bounds[1] >= y
                && instance.bounds[2] <= x + width
                && instance.bounds[3] <= y + height
        })
}

const fn source_target(source: Source) -> Target {
    match source {
        Source::Scratch(i) => Target::Scratch(i),
        Source::Backdrop { group, region } => Target::Backdrop { group, region },
        Source::Projected(key) => Target::Projected(key),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
