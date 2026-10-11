use super::*;
use crate::render::instance::{Instance, KIND_SPAN};
use crate::render::lower::{DrawRange, Pass, ShaderVariant};

fn pass(target: Target, clear: bool, source: Option<Source>, instance: u32) -> Pass {
    Pass {
        target,
        clear: clear.then_some([0.0; 4]),
        ranges: vec![DrawRange {
            source,
            image: None,
            mask: None,
            pipeline: PipelineKind::SrcOver,
            variant: ShaderVariant::Full,
            instances: instance..instance + 1,
        }],
        space: cherenkov::BlendSpace::Linear,
        region: [0, 0, 16, 16],
        backdrop_copy: None,
        capture: None,
    }
}

fn frame(passes: Vec<Pass>) -> Frame {
    let mut frame = Frame::default();
    frame.instances = passes
        .iter()
        .map(|pass| {
            let mut instance = Instance::new(KIND_SPAN);
            instance.bounds = [0.0, 0.0, 16.0, 16.0];
            if pass.ranges[0].source.is_some() {
                instance.meta[1] = PAINT_TEXTURE;
            }
            instance
        })
        .collect();
    frame.passes = passes;
    frame
}

#[test]
fn scratch_generations_do_not_alias_lifetimes() {
    let frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
        pass(Target::Scratch(0), true, None, 3),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 4),
    ]);
    let plan = ExecutionPlan::build(&frame, wgpu::TextureFormat::Rgba16Float, 2);
    let first = plan.targets[1];
    let second = plan.targets[3];
    assert_ne!(first, second);
    assert_eq!(plan.images[first.0].generation, 0);
    assert_eq!(plan.images[second.0].generation, 1);
    assert!(!plan.materialized_pass(1));
    assert!(!plan.materialized_pass(3));
    assert_eq!(plan.epoch_at(1).unwrap().passes, 1..3);
    assert_eq!(plan.epoch_at(3).unwrap().passes, 3..5);
}

#[test]
fn attachment_budget_materializes_outer_image_without_recomputing() {
    let frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Scratch(1), true, None, 2),
        pass(Target::Scratch(0), false, Some(Source::Scratch(1)), 3),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 4),
    ]);
    let plan = ExecutionPlan::build(&frame, wgpu::TextureFormat::Rgba16Float, 2);
    assert!(plan.materialized_pass(1));
    assert!(!plan.materialized_pass(2));
    assert_eq!(plan.epoch_at(2).unwrap().passes, 2..4);
    let plan = ExecutionPlan::build(&frame, wgpu::TextureFormat::Rgba16Float, 4);
    assert!(!plan.materialized_pass(1));
    assert!(!plan.materialized_pass(2));
    assert_eq!(plan.epoch_at(1).unwrap().passes, 1..5);
}

#[test]
fn neighborhood_reads_and_overlapping_snapshots_materialize() {
    let mut frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ]);
    frame.instances[2].meta[1] = crate::render::instance::PAINT_BACKDROP;
    let plan = ExecutionPlan::build(&frame, wgpu::TextureFormat::Rgba16Float, 4);
    assert!(plan.epoch_at(1).is_none());
    frame.instances[2].meta[1] = PAINT_TEXTURE;
    frame.passes[2].backdrop_copy = Some([0, 0, 16, 16]);
    let plan = ExecutionPlan::build(&frame, wgpu::TextureFormat::Rgba16Float, 4);
    assert!(plan.epoch_at(1).is_some());
    frame.instances.push(frame.instances[2]);
    frame.passes[2].ranges[0].instances.end = 4;
    let plan = ExecutionPlan::build(&frame, wgpu::TextureFormat::Rgba16Float, 4);
    assert!(plan.epoch_at(1).is_none());
}

#[test]
fn cached_paint_updates_preserve_lifetimes_and_motion_rechecks_locality() {
    let mut frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ]);
    let mut cache = crate::render::composite::cache::Cache::default();
    let mut plan = ExecutionPlan::default();
    let format = wgpu::TextureFormat::Rgba16Float;
    assert!(cache.update(&mut plan, &frame, format, 2));
    let image = plan.targets[1];
    frame.instances[1].color = [1.5, 0.3, 0.0, 0.8];
    frame.instances[2].params[1] = 0.5;
    assert!(!cache.update(&mut plan, &frame, format, 2));
    assert_eq!(plan.targets[1], image);
    assert!(plan.epoch_at(1).is_some());
    // The read no longer corresponds to this framebuffer pixel. Reusing
    // the old epoch would fetch a different pixel than the canonical path.
    frame.instances[2].grad[0] = 1.0;
    assert!(!cache.update(&mut plan, &frame, format, 2));
    assert_eq!(plan.targets[1], image);
    assert!(plan.epoch_at(1).is_none());
    assert!(plan.materialized_pass(1));
}
