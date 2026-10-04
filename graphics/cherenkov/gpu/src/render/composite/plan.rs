//! Image lifetimes derived from the completed canonical frame.
//!
//! A scratch depth is an allocation address, not an image identity. Every
//! clear starts a new generation; reads name that generation before any
//! physical attachment is assigned.

use std::ops::Range;

use super::super::instance::PAINT_TEXTURE;
use super::super::lower::{DrawRange, Frame, ImageSource, PipelineKind, Source, Target};

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
    pub materialized: bool,
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
    pub backdrop: Option<ReadKind>,
}

/// Slot zero is the persistent output. Other slots have no device-memory
/// backing and are cleared at each logical image's original begin point.
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

    pub fn build(frame: &Frame, format: wgpu::TextureFormat, slots: u8) -> Self {
        let mut plan = Self::default();
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
                        format
                    } else {
                        wgpu::TextureFormat::Rgba16Float
                    },
                    producer: Some(index),
                    last_consumer: index,
                    materialized: true,
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
                            materialized: true,
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
        plan.partition(frame, slots);
        plan
    }

    pub fn update_placement(&mut self, frame: &Frame, slots: u8) {
        for (index, pass) in frame.passes.iter().enumerate() {
            let image = &mut self.images[self.targets[index].0];
            image.region = pass.region;
            image.space = pass.space;
            image.materialized = true;
        }
        self.segments.clear();
        self.partition(frame, slots);
    }

    fn partition(&mut self, frame: &Frame, slots: u8) {
        let plan = self;
        plan.native.clear();
        plan.native.resize(frame.passes.len(), None);
        let mut index = 0;
        while index < frame.passes.len() {
            if slots >= 2
                && let Some(epoch) = plan.epoch(frame, index, slots)
            {
                for op in &epoch.ops {
                    let id = plan.targets[op.pass];
                    if id != epoch.output {
                        plan.images[id.0].materialized = false;
                    }
                }
                index = epoch.passes.end;
                plan.native[epoch.passes.clone()].fill(Some(plan.segments.len()));
                plan.segments.push(Segment::NativeEpoch(epoch));
            } else {
                if let Some(Segment::Portable(range)) = plan.segments.last_mut() {
                    range.end = index + 1;
                } else {
                    plan.segments.push(Segment::Portable(index..index + 1));
                }
                index += 1;
            }
        }
    }

    fn epoch(&self, frame: &Frame, start: usize, limit: u8) -> Option<TileEpoch> {
        let first = &self.images[self.targets[start].0];
        if !matches!(first.target, Target::Scratch(_)) || first.producer != Some(start) {
            return None;
        }
        let end = first.last_consumer;
        if end == start {
            return None;
        }
        let output = self.targets[end];
        let destination = &self.images[output.0];
        if output == self.targets[start]
            || !matches!(destination.target, Target::Part(_) | Target::Scratch(_))
            || destination.format != wgpu::TextureFormat::Rgba16Float
        {
            return None;
        }
        // A filtered result, custom effect, retained image, external operation
        // or snapshot with overlapping consumers cannot cross a tile epoch.
        let mut assigned = vec![(output, 0u8)];
        let mut slots = 1;
        let mut ops = Vec::new();
        for index in start..=end {
            let pass = &frame.passes[index];
            let id = self.targets[index];
            let image = &self.images[id.0];
            if materialization_boundary(frame, index)
                || pass.capture.is_some()
                || image.format != wgpu::TextureFormat::Rgba16Float
                || image.space != pass.space
                || !contains(destination.region, image.region)
                || pass.ranges.iter().any(|range| {
                    !matches!(
                        range.pipeline,
                        PipelineKind::SrcOver | PipelineKind::Replace
                    ) || matches!(
                        range.image,
                        Some(ImageSource::Content(_) | ImageSource::Shader(_))
                    )
                })
            {
                return None;
            }
            // A single canonical composite quad cannot observe its own earlier
            // fragments. Only that proof permits replacing a frozen snapshot
            // with the live destination; all other snapshots materialize.
            if pass.backdrop_copy.is_some()
                && (pass.ranges.len() != 1
                    || pass.ranges[0].instances.len() != 1
                    || self.reads[index][0].as_ref()?.kind != ReadKind::CorrespondingPixel)
            {
                return None;
            }
            let begin = image.producer == Some(index);
            let target = if let Some((_, slot)) = assigned.iter().find(|(i, _)| *i == id) {
                *slot
            } else {
                if !begin
                    || image.last_consumer > end
                    || !matches!(image.target, Target::Scratch(_))
                    || pass.clear != Some([0.0; 4])
                {
                    return None;
                }
                let slot = (1..limit).find(|slot| {
                    assigned
                        .iter()
                        .all(|(i, s)| s != slot || self.images[i.0].last_consumer < index)
                })?;
                slots = slots.max(slot + 1);
                assigned.push((id, slot));
                slot
            };
            let sources = self.reads[index]
                .iter()
                .zip(&pass.ranges)
                .map(|(read, range)| match read {
                    None => Some(None),
                    Some(read) if read.kind == ReadKind::CorrespondingPixel => {
                        let (_, slot) = assigned.iter().find(|(i, _)| *i == read.image)?;
                        let source = &self.images[read.image.0];
                        if !corresponds(frame, range, source.region) {
                            return None;
                        }
                        // Attachment fetch and texture fetch must identify the
                        // same pixel, including the source's virtual extent.
                        Some(Some(*slot))
                    }
                    Some(_) => None,
                })
                .collect::<Option<Vec<_>>>()?;
            ops.push(TileOp {
                pass: index,
                target,
                sources,
                begin,
                backdrop: pass.backdrop_copy.map(|_| ReadKind::FrozenSnapshot),
            });
        }
        Some(TileEpoch {
            passes: start..end + 1,
            output,
            region: destination.region,
            slots,
            ops,
        })
    }

    pub fn materialized_pass(&self, pass: usize) -> bool {
        self.images[self.targets[pass].0].materialized
    }

    pub fn epoch_at(&self, pass: usize) -> Option<&TileEpoch> {
        self.native[pass].and_then(|index| match &self.segments[index] {
            Segment::NativeEpoch(epoch) if epoch.passes.start == pass => Some(epoch),
            _ => None,
        })
    }

    pub fn contains_native_pass(&self, pass: usize) -> bool {
        self.native[pass].is_some()
    }

    pub fn attachment_origin(&self, pass: usize) -> Option<[u32; 2]> {
        self.native[pass].map(|index| {
            let Segment::NativeEpoch(epoch) = &self.segments[index] else {
                unreachable!("native pass index names its epoch")
            };
            [epoch.region[0], epoch.region[1]]
        })
    }
}

pub fn read_kind(frame: &Frame, range: &DrawRange) -> ReadKind {
    if range.source.is_none() || matches!(range.pipeline, PipelineKind::Effect(_)) {
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

pub fn materialization_boundary(frame: &Frame, index: usize) -> bool {
    frame.filters.iter().any(|(i, _)| *i == index)
        || frame.shadows.iter().any(|(i, _)| *i == index)
        || frame.mips.iter().any(|(i, _)| *i == index)
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
