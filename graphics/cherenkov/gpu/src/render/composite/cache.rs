//! Exact structural keys: changing paint values never repeats lifetime analysis.

use super::plan::{ExecutionPlan, ReadKind};
use crate::render::lower::{Frame, ImageSource, Pass, PipelineKind, Source, Target};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Draw {
    source: Option<Source>,
    read: ReadKind,
    opaque: bool,
    count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PassShape {
    target: Target,
    clear: bool,
    capture: Option<Target>,
    backdrop: bool,
    boundary: bool,
    ranges: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Placement {
    region: [u32; 4],
    space: cherenkov::BlendSpace,
    transparent_clear: bool,
}

#[derive(Default)]
pub struct Cache {
    passes: Vec<PassShape>,
    draws: Vec<Draw>,
    placements: Vec<Placement>,
    samples: Vec<[u32; 6]>,
    settings: Option<(wgpu::TextureFormat, u8)>,
}

impl Cache {
    pub const fn bytes(&self) -> u64 {
        (self.passes.capacity() * std::mem::size_of::<PassShape>()
            + self.draws.capacity() * std::mem::size_of::<Draw>()
            + self.placements.capacity() * std::mem::size_of::<Placement>()
            + self.samples.capacity() * std::mem::size_of::<[u32; 6]>()) as u64
    }

    /// Returns whether lifetime analysis changed. Geometry-only updates
    /// retain image identities and recompute attachment eligibility.
    pub fn update(
        &mut self,
        plan: &mut ExecutionPlan,
        frame: &Frame,
        format: wgpu::TextureFormat,
        slots: u8,
    ) -> bool {
        let pass_shapes = || {
            frame
                .passes
                .iter()
                .enumerate()
                .map(|(index, pass)| PassShape {
                    target: pass.target,
                    clear: pass.clear.is_some(),
                    capture: pass.capture.map(|capture| capture.copy_from),
                    backdrop: pass.backdrop_copy.is_some(),
                    ranges: pass.ranges.len(),
                    boundary: super::plan::materialization_boundary(frame, index),
                })
        };
        let draws = || {
            frame
                .passes
                .iter()
                .flat_map(|pass| pass.ranges.iter())
                .map(|range| Draw {
                    source: range.source,
                    read: super::plan::read_kind(frame, range),
                    opaque: !matches!(
                        range.pipeline,
                        PipelineKind::SrcOver | PipelineKind::Replace
                    ) || matches!(
                        range.image,
                        Some(ImageSource::Content(_) | ImageSource::Shader(_))
                    ),
                    count: range.instances.len(),
                })
        };
        let changed = self.settings != Some((format, slots))
            || !self.passes.iter().copied().eq(pass_shapes())
            || !self.draws.iter().copied().eq(draws());
        if changed {
            self.passes.clear();
            self.passes.extend(pass_shapes());
            self.draws.clear();
            self.draws.extend(draws());
            self.settings = Some((format, slots));
            *plan = ExecutionPlan::build(frame, format, slots);
        }
        let placements = || frame.passes.iter().map(placement);
        let samples = || {
            frame
                .passes
                .iter()
                .flat_map(|pass| &pass.ranges)
                .filter(|range| range.source.is_some())
                .flat_map(|range| {
                    &frame.instances[range.instances.start as usize..range.instances.end as usize]
                })
                .map(|instance| {
                    [
                        instance.grad[0],
                        instance.grad[1],
                        instance.bounds[0],
                        instance.bounds[1],
                        instance.bounds[2],
                        instance.bounds[3],
                    ]
                    .map(f32::to_bits)
                })
        };
        let moved = !self.placements.iter().copied().eq(placements())
            || !self.samples.iter().copied().eq(samples());
        if changed || moved {
            self.placements.clear();
            self.placements.extend(placements());
            self.samples.clear();
            self.samples.extend(samples());
            if !changed {
                plan.update_placement(frame, slots);
            }
        }
        changed
    }
}

fn placement(pass: &Pass) -> Placement {
    Placement {
        region: pass.region,
        space: pass.space,
        transparent_clear: pass
            .clear
            .is_some_and(|clear| clear.iter().all(|v| *v == 0.0)),
    }
}
