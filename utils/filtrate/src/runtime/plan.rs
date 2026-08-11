//! Stage collection and pass planning: fusing color fragments, assigning
//! scratch slots, and resolving each pass's texture bindings.

extern crate alloc;

use alloc::vec::Vec;

use filtrate_core::{Filter, StageCollector};

use crate::effect::EffectSetupError;

use super::pass::{ColorTarget, PassBindingPlan, PassTextureSource, SCRATCH_SLOT_COUNT};
use super::shader::storage_format_to_wgsl;

#[derive(Debug, Clone, Copy)]
pub(super) enum AtomicStageKind {
    ColorFragment(&'static str),
    SpatialShader(&'static str),
    SpatialShaderWithOriginal(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AtomicStage {
    pub(super) kind: AtomicStageKind,
    pub(super) param_count: usize,
}

#[derive(Debug, Clone)]
pub(super) enum PlannedPassKind {
    Color {
        fragments: alloc::string::String,
    },
    Spatial {
        shader: &'static str,
        original_input: bool,
    },
}

#[derive(Debug, Clone)]
pub(super) struct PlannedPass {
    pub(super) kind: PlannedPassKind,
    pub(super) param_offset: usize,
    pub(super) param_count: usize,
}

pub(super) struct FilterSetupPlan {
    pub(super) passes: Vec<PlannedPass>,
    pub(super) scratch_formats: Vec<wgpu::TextureFormat>,
}

pub(super) fn fuse_stages(stages: &[AtomicStage]) -> Result<Vec<PlannedPass>, EffectSetupError> {
    if stages.is_empty() {
        return Err(EffectSetupError::EmptyGraph);
    }

    let mut passes: Vec<PlannedPass> = Vec::with_capacity(stages.len());
    let mut param_offset = 0usize;

    for stage in stages {
        match stage.kind {
            AtomicStageKind::ColorFragment(fragment) => {
                let fused_into_last = match passes.last_mut() {
                    Some(PlannedPass {
                        kind: PlannedPassKind::Color { fragments },
                        param_count,
                        ..
                    }) => {
                        fragments.push_str(fragment);
                        fragments.push('\n');
                        *param_count += stage.param_count;
                        true
                    }
                    _ => false,
                };
                if !fused_into_last {
                    let mut fused = alloc::string::String::new();
                    fused.push_str(fragment);
                    fused.push('\n');
                    passes.push(PlannedPass {
                        kind: PlannedPassKind::Color { fragments: fused },
                        param_offset,
                        param_count: stage.param_count,
                    });
                }
            }
            AtomicStageKind::SpatialShader(shader) => {
                passes.push(PlannedPass {
                    kind: PlannedPassKind::Spatial {
                        shader,
                        original_input: false,
                    },
                    param_offset,
                    param_count: stage.param_count,
                });
            }
            AtomicStageKind::SpatialShaderWithOriginal(shader) => {
                passes.push(PlannedPass {
                    kind: PlannedPassKind::Spatial {
                        shader,
                        original_input: true,
                    },
                    param_offset,
                    param_count: stage.param_count,
                });
            }
        }
        param_offset += stage.param_count;
    }

    Ok(passes)
}

pub(super) fn pick_scratch_slot(
    forbidden: &[PassTextureSource],
) -> Result<usize, EffectSetupError> {
    (0..SCRATCH_SLOT_COUNT)
        .find(|slot| !forbidden.contains(&PassTextureSource::Scratch(*slot)))
        .ok_or(EffectSetupError::PlannerInvariant(
            "no scratch slot available that does not alias a bound texture",
        ))
}

/// Index of the final pass iff it is a plain spatial pass and the output
/// format can be written as a storage texture — the pass then also compiles
/// a direct-output pipeline so render can skip the final blit.
/// `spatial_shader_with_original` passes are excluded: their original
/// binding may be output-sized scratch while a swapchain output rotates.
pub(super) fn final_direct_output_pass_index(
    planned: &[PlannedPass],
    output_format: wgpu::TextureFormat,
) -> Option<usize> {
    if storage_format_to_wgsl(output_format).is_err() {
        return None;
    }
    let (idx, last) = planned.iter().enumerate().next_back()?;
    match last.kind {
        PlannedPassKind::Spatial {
            original_input: false,
            ..
        } => Some(idx),
        _ => None,
    }
}

pub(super) fn plan_runtime_bindings(
    planned: &[PlannedPass],
) -> Result<(Vec<PassBindingPlan>, Option<usize>), EffectSetupError> {
    if planned.is_empty() {
        return Err(EffectSetupError::EmptyGraph);
    }

    let mut plans = Vec::with_capacity(planned.len());
    let mut source = PassTextureSource::Input;
    // The source read by the previous pass — a `spatial_shader_with_original`
    // pass binds this as the "original" (the texture that fed its filter's
    // first stage).
    let mut previous_source: Option<PassTextureSource> = None;

    for (idx, pass) in planned.iter().enumerate() {
        let is_last = idx + 1 == planned.len();
        match &pass.kind {
            PlannedPassKind::Color { .. } => {
                let pass_source = source;
                let target = if is_last {
                    ColorTarget::Output
                } else {
                    let slot = pick_scratch_slot(&[pass_source])?;
                    source = PassTextureSource::Scratch(slot);
                    ColorTarget::Scratch(slot)
                };
                plans.push(PassBindingPlan::Color {
                    source: pass_source,
                    target,
                });
                previous_source = Some(pass_source);
            }
            PlannedPassKind::Spatial { original_input, .. } => {
                let original =
                    original_input.then(|| previous_source.unwrap_or(PassTextureSource::Input));
                // The write target must alias neither the sampled source
                // nor the retained original.
                let mut forbidden = alloc::vec![source];
                if let Some(original) = original {
                    forbidden.push(original);
                }
                let target_scratch = pick_scratch_slot(&forbidden)?;
                plans.push(PassBindingPlan::Spatial {
                    source,
                    target_scratch,
                    original,
                });
                previous_source = Some(source);
                source = PassTextureSource::Scratch(target_scratch);
            }
        }
    }

    let blit_source_scratch = match planned.last().map(|pass| &pass.kind) {
        Some(PlannedPassKind::Spatial { .. }) => match source {
            PassTextureSource::Scratch(slot) => Some(slot),
            PassTextureSource::Input => {
                return Err(EffectSetupError::PlannerInvariant(
                    "spatial pipeline planner produced invalid blit source",
                ));
            }
        },
        Some(PlannedPassKind::Color { .. }) | None => None,
    };

    Ok((plans, blit_source_scratch))
}

#[derive(Default)]
pub(super) struct StageBuffer {
    pub(super) stages: Vec<AtomicStage>,
}

impl StageBuffer {
    fn into_inner(self) -> Vec<AtomicStage> {
        self.stages
    }
}

impl StageCollector for StageBuffer {
    fn color_fragment(&mut self, source: &'static str, param_count: usize) {
        self.stages.push(AtomicStage {
            kind: AtomicStageKind::ColorFragment(source),
            param_count,
        });
    }
    fn spatial_shader(&mut self, source: &'static str, param_count: usize) {
        self.stages.push(AtomicStage {
            kind: AtomicStageKind::SpatialShader(source),
            param_count,
        });
    }

    fn spatial_shader_with_original(&mut self, source: &'static str, param_count: usize) {
        self.stages.push(AtomicStage {
            kind: AtomicStageKind::SpatialShaderWithOriginal(source),
            param_count,
        });
    }
}

pub(super) fn collect_filter_stages<F: Filter>(filter: &F) -> Vec<AtomicStage> {
    let mut buffer = StageBuffer::default();
    filter.collect_stages(&mut buffer);
    buffer.into_inner()
}
