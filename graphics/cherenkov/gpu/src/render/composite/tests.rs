use std::ops::Range;

use super::*;
use crate::render::composite::plan::{Formats, Limits, Ordering};
use crate::render::composition::Materialization;
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
            gpu: None,
        }],
        space: cherenkov::BlendSpace::Linear,
        region: [0, 0, 16, 16],
        backdrop_copy: None,
        capture: None,
        tier_ranges: [Range::default(), Range::default()],
    }
}

/// A tile executor with `slots` attachment slots, the engine's
/// attachment format, and per-pixel ordering within a pass — the Apple
/// backend's shape, with the slot count under test.
fn limits(slots: u8) -> Limits {
    Limits {
        slots,
        format: wgpu::TextureFormat::Rgba16Float,
        ordering: Ordering::PerPixel,
    }
}

/// A working-format surface with isolation scratches in `scratch`.
const fn formats(scratch: wgpu::TextureFormat) -> Formats {
    Formats {
        surface: wgpu::TextureFormat::Rgba16Float,
        scratch,
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
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(2));
    let first = plan.targets[1];
    let second = plan.targets[3];
    assert_ne!(first, second);
    assert_eq!(plan.images[first.0].generation, 0);
    assert_eq!(plan.images[second.0].generation, 1);
    assert_eq!(plan.materialization(1), None);
    assert_eq!(plan.materialization(3), None);
    // Both generations store into the same part image, back to back, so
    // one native pass carries them and the second reuses the first's slot.
    let epoch = plan.epoch_at(1).expect("one epoch covers both generations");
    assert_eq!(epoch.passes, 1..5);
    assert_eq!(epoch.slots, 2);
    let op = |pass: usize| {
        epoch
            .ops
            .iter()
            .find(|op| op.pass == pass)
            .expect("every pass of the epoch has an op")
    };
    assert_eq!(op(1).target, op(3).target);
    assert!(op(3).begin);
}

/// Intervals that store into the same logical image back to back are one
/// native pass: the output never leaves the executor's slot zero between
/// them. A clear of that output ends the image, and the pass that clears
/// it is portable, so neither condition holds across it.
#[test]
fn a_cleared_output_separates_two_native_passes() {
    let frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
        pass(Target::Part(0), true, None, 3),
        pass(Target::Scratch(0), true, None, 4),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 5),
    ]);
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    assert_ne!(plan.targets[2], plan.targets[5]);
    assert_eq!(plan.epoch_at(1).expect("first epoch").passes, 1..3);
    assert_eq!(plan.epoch_at(4).expect("second epoch").passes, 4..6);
    assert_eq!(plan.epochs().count(), 2);
    assert!(!plan.contains_native_pass(3));
}

/// A slot a coalesced epoch reuses still holds the previous image's
/// pixels. The clear the executor draws covers every region the new
/// image is written over, which with `corresponds` bounding its reads is
/// what that image's own clear answered before the intervals shared a
/// pass.
#[test]
fn a_reused_slot_clears_every_region_its_image_covers() {
    let mut passes = vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
        // The second generation opens over a 4×4 corner and is then
        // written again over 8×8, two passes before it is read.
        pass(Target::Scratch(0), true, None, 3),
        pass(Target::Scratch(0), false, None, 4),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 5),
    ];
    passes[3].region = [0, 0, 4, 4];
    passes[4].region = [0, 0, 8, 8];
    let mut frame = frame(passes);
    // The read corresponds to the image's own region, as `epoch` demands.
    frame.instances[5].bounds = [0.0, 0.0, 4.0, 4.0];
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    let epoch = plan.epoch_at(1).expect("one epoch covers both generations");
    assert_eq!(epoch.passes, 1..6);
    let op = |pass: usize| {
        epoch
            .ops
            .iter()
            .find(|op| op.pass == pass)
            .expect("every pass of the epoch has an op")
    };
    assert_eq!(op(1).target, op(3).target);
    // The reuse clears all 8×8 the epoch writes the image over, not just
    // the 4×4 the opening draw covers.
    assert!(op(3).begin);
    assert_eq!(op(3).clear_region, [0, 0, 8, 8]);
    assert_eq!(plan.materialization(3), None);
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
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(2));
    assert_eq!(plan.materialization(1), Some(Materialization::SlotLimit));
    assert_eq!(plan.materialization(2), None);
    assert_eq!(plan.epoch_at(2).unwrap().passes, 2..4);
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    assert_eq!(plan.materialization(1), None);
    assert_eq!(plan.materialization(2), None);
    assert_eq!(plan.epoch_at(1).unwrap().passes, 1..5);
}

/// A nested group's slot is free the moment it composites into its
/// parent, so two siblings of one parent share one slot and the epoch
/// binds two attachments, not three.
#[test]
fn sibling_groups_reuse_one_slot() {
    let frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Scratch(1), true, None, 2),
        pass(Target::Scratch(0), false, Some(Source::Scratch(1)), 3),
        pass(Target::Scratch(1), true, None, 4),
        pass(Target::Scratch(0), false, Some(Source::Scratch(1)), 5),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 6),
    ]);
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    let epoch = plan.epoch_at(1).expect("one epoch covers the whole tree");
    assert_eq!(epoch.passes, 1..7);
    // Slot zero stores the part; the parent group and the two siblings
    // in turn occupy the other two.
    assert_eq!(epoch.slots, 3);
    let slot = |pass: usize| {
        epoch
            .ops
            .iter()
            .find(|op| op.pass == pass)
            .expect("every pass of the epoch has an op")
            .target
    };
    assert_eq!(slot(6), 0);
    assert_eq!(slot(1), slot(3));
    assert_eq!(slot(1), slot(5));
    // The second sibling is a new logical image that lands in the slot
    // the first one vacated at pass 3.
    assert_ne!(plan.targets[2], plan.targets[4]);
    assert_eq!(slot(2), slot(4));
    assert_ne!(slot(2), slot(1));
    for pass in 1..6 {
        assert_eq!(plan.materialization(pass), None);
    }
}

/// With no executor a backend supplies no limits, so nothing is planned:
/// every pass keeps its backed attachment and the frame is attributed to
/// the missing executor (#2549).
#[test]
fn an_unplanned_frame_names_the_missing_executor() {
    let plan = ExecutionPlan::default();
    assert!(!plan.is_planned());
    assert_eq!(plan.bytes(), 0);
    for pass in 0..4 {
        assert_eq!(
            plan.materialization(pass),
            Some(Materialization::NoExecutor)
        );
        assert!(!plan.contains_native_pass(pass));
        assert!(plan.epoch_at(pass).is_none());
        assert_eq!(plan.attachment_origin(pass), None);
    }
    assert_eq!(plan.epochs().count(), 0);
}

/// An executor that orders slot accesses only between passes cannot hold
/// an epoch at all: every draw of one would read what an earlier draw of
/// the same pass wrote.
#[test]
fn per_pass_ordering_admits_no_epoch() {
    let frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ]);
    let plan = ExecutionPlan::build(
        &frame,
        formats(wgpu::TextureFormat::Rgba16Float),
        Limits {
            ordering: Ordering::PerPass,
            ..limits(4)
        },
    );
    assert!(plan.is_planned());
    assert_eq!(plan.epochs().count(), 0);
    assert_eq!(plan.materialization(1), Some(Materialization::Ordering));
}

#[test]
fn neighborhood_reads_and_overlapping_snapshots_materialize() {
    let mut frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ]);
    frame.instances[2].meta[1] = crate::render::instance::PAINT_BACKDROP;
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    assert!(plan.epoch_at(1).is_none());
    assert_eq!(
        plan.materialization(1),
        Some(Materialization::NeighborhoodRead)
    );
    frame.instances[2].meta[1] = PAINT_TEXTURE;
    frame.passes[2].backdrop_copy = Some([0, 0, 16, 16]);
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    assert!(plan.epoch_at(1).is_some());
    frame.instances.push(frame.instances[2]);
    frame.passes[2].ranges[0].instances.end = 4;
    let plan = ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4));
    assert!(plan.epoch_at(1).is_none());
    assert_eq!(
        plan.materialization(1),
        Some(Materialization::FrozenSnapshot)
    );
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
    let format = formats(wgpu::TextureFormat::Rgba16Float);
    assert!(cache.update(&mut plan, &frame, format, Ok(limits(2))));
    let image = plan.targets[1];
    frame.instances[1].color = [1.5, 0.3, 0.0, 0.8];
    frame.instances[2].params[1] = 0.5;
    assert!(!cache.update(&mut plan, &frame, format, Ok(limits(2))));
    assert_eq!(plan.targets[1], image);
    assert!(plan.epoch_at(1).is_some());
    // The read no longer corresponds to this framebuffer pixel. Reusing
    // the old epoch would fetch a different pixel than the canonical path.
    frame.instances[2].grad[0] = 1.0;
    assert!(!cache.update(&mut plan, &frame, format, Ok(limits(2))));
    assert_eq!(plan.targets[1], image);
    assert!(plan.epoch_at(1).is_none());
    assert_eq!(
        plan.materialization(1),
        Some(Materialization::NeighborhoodRead)
    );
    frame.instances[2].grad[0] = 0.0;
    assert!(!cache.update(&mut plan, &frame, format, Ok(limits(2))));
    assert!(plan.epoch_at(1).is_some());
    assert!(cache.update(&mut plan, &frame, format, Err(Materialization::NoExecutor)));
    assert_eq!(plan.epochs().count(), 0);
    assert_eq!(plan.targets[1], image);
    frame.passes[1].region = [1, 1, 8, 8];
    assert!(!cache.update(&mut plan, &frame, format, Err(Materialization::NoExecutor)));
    assert_eq!(plan.images[image.0].region, [1, 1, 8, 8]);
    assert_eq!(
        plan.reads[2][0].as_ref().expect("scratch read").writer,
        Some(1)
    );
    assert_eq!(plan.materialization(1), Some(Materialization::NoExecutor));
}

/// Every reason the planner can name, each from the frame shape that
/// provokes it (#2547). The three remaining reasons are asserted where
/// their behaviour is already tested: `SlotLimit` in
/// `attachment_budget_materializes_outer_image_without_recomputing`,
/// `NeighborhoodRead` and `FrozenSnapshot` in
/// `neighborhood_reads_and_overlapping_snapshots_materialize`.
#[test]
fn every_materialization_reason_is_attributed() {
    let format = wgpu::TextureFormat::Rgba16Float;
    let fuses = vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ];

    // A surface part is the frame's output: never an epoch's intermediate.
    let plan = ExecutionPlan::build(&frame(fuses.clone()), formats(format), limits(4));
    assert_eq!(plan.materialization(0), Some(Materialization::Persistent));
    assert_eq!(plan.materialization(1), None);

    // Written and never read: no producer-to-consumer interval at all.
    let plan = ExecutionPlan::build(
        &frame(vec![
            pass(Target::Part(0), true, None, 0),
            pass(Target::Scratch(0), true, None, 1),
        ]),
        formats(format),
        limits(4),
    );
    assert_eq!(plan.materialization(1), Some(Materialization::NoInterval));

    // The last consumer writes the image itself, so the interval has no
    // other image to store into.
    let plan = ExecutionPlan::build(
        &frame(vec![
            pass(Target::Part(0), true, None, 0),
            pass(Target::Scratch(0), true, None, 1),
            pass(Target::Scratch(0), false, Some(Source::Scratch(0)), 2),
        ]),
        formats(format),
        limits(4),
    );
    assert_eq!(
        plan.materialization(1),
        Some(Materialization::OutputNotEligible)
    );

    // An inner image whose last consumer is past the interval's end
    // escapes it.
    let plan = ExecutionPlan::build(
        &frame(vec![
            pass(Target::Part(0), true, None, 0),
            pass(Target::Scratch(0), true, None, 1),
            pass(Target::Scratch(1), true, None, 2),
            pass(Target::Scratch(0), false, Some(Source::Scratch(1)), 3),
            pass(Target::Part(0), false, Some(Source::Scratch(0)), 4),
            pass(Target::Part(0), false, Some(Source::Scratch(1)), 5),
        ]),
        formats(format),
        limits(4),
    );
    assert_eq!(plan.materialization(1), Some(Materialization::External));

    // A scratch format the tile attachments cannot carry.
    let plan = ExecutionPlan::build(
        &frame(fuses.clone()),
        formats(wgpu::TextureFormat::Rgba8Unorm),
        limits(4),
    );
    assert_eq!(plan.materialization(1), Some(Materialization::Format));

    // The second write composites in a different space than the image it
    // continues: fusing would move the conversion point.
    let mut shifted = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Scratch(0), false, None, 2),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 3),
    ]);
    shifted.passes[2].space = cherenkov::BlendSpace::SrgbEncoded;
    let plan = ExecutionPlan::build(&shifted, formats(format), limits(4));
    assert_eq!(plan.materialization(1), Some(Materialization::BlendSpace));

    // The intermediate covers pixels the epoch's attachment region, which
    // is its output's, does not.
    let mut wider = frame(fuses.clone());
    wider.passes[1].region = [0, 0, 32, 32];
    let plan = ExecutionPlan::build(&wider, formats(format), limits(4));
    assert_eq!(plan.materialization(1), Some(Materialization::Region));

    // A blend the on-chip executor does not implement.
    let mut projective = frame(fuses);
    projective.passes[2].ranges[0].pipeline = PipelineKind::Projective { replace: false };
    let plan = ExecutionPlan::build(&projective, formats(format), limits(4));
    assert_eq!(
        plan.materialization(1),
        Some(Materialization::UnsupportedOp)
    );
}

#[test]
fn frame_report_attributes_the_epoch_and_the_traffic_around_it() {
    use crate::render::composition::{self, AttachmentBytes, ImageKind};

    // 16x16 rgba16float: the fixture's region in the attachment format.
    const REGION_BYTES: u64 = 16 * 16 * 8;

    let format = wgpu::TextureFormat::Rgba16Float;
    let fused = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ]);
    let plan = ExecutionPlan::build(&fused, formats(format), limits(4));
    let report = composition::frame_report(
        7,
        3,
        &fused,
        &plan,
        AttachmentBytes {
            peak_reserved_bytes: REGION_BYTES,
            peak_committed_bytes: Some(0),
            peak_backed_bytes: 4 * REGION_BYTES,
        },
        Vec::new(),
    );

    assert_eq!((report.frame, report.surface, report.passes), (7, 3, 3));
    assert_eq!(report.epochs.len(), 1);
    assert_eq!(report.epochs[0].passes, 1..3);
    assert_eq!(report.epochs[0].slots, 2);
    assert_eq!(report.epochs[0].on_chip_images, 1);
    // The scratch stays in a slot; only the part reaches memory.
    assert_eq!(report.on_chip.len(), 1);
    assert_eq!(report.on_chip[0].kind, ImageKind::Scratch);
    assert_eq!(report.on_chip[0].logical_bytes, REGION_BYTES);
    assert_eq!(report.materialized.len(), 1);
    assert_eq!(report.materialized[0].kind, ImageKind::Part);
    assert_eq!(report.materialized[0].reason, Materialization::Persistent);
    assert_eq!(report.attachments.peak_reserved_bytes, REGION_BYTES);
    assert_eq!(report.attachments.peak_committed_bytes, Some(0));
    assert_eq!(report.attachments.peak_backed_bytes, 4 * REGION_BYTES);
    // One intermediate written, read once, ordered by the epoch itself.
    assert_eq!(report.traffic.logical_bytes_written, REGION_BYTES);
    assert_eq!(report.traffic.logical_bytes_read, REGION_BYTES);
    assert_eq!(report.traffic.logical_bytes_copied, 0);
    assert_eq!(report.traffic.barriers, 0);
    assert!(report.pipelines.is_empty());

    // The same frame with a neighborhood read: the planner materializes
    // the intermediate, and the read of it becomes an ordered boundary.
    let mut blurred = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), false, Some(Source::Scratch(0)), 2),
    ]);
    blurred.instances[2].meta[1] = crate::render::instance::PAINT_BACKDROP;
    let plan = ExecutionPlan::build(&blurred, formats(format), limits(4));
    let report = composition::frame_report(
        8,
        3,
        &blurred,
        &plan,
        AttachmentBytes::default(),
        Vec::new(),
    );
    assert!(report.epochs.is_empty());
    assert!(report.on_chip.is_empty());
    let scratch = report
        .materialized
        .iter()
        .find(|image| image.kind == ImageKind::Scratch)
        .expect("the intermediate is materialized");
    assert_eq!(scratch.reason, Materialization::NeighborhoodRead);
    assert_eq!(scratch.logical_bytes, REGION_BYTES);
    assert_eq!(report.traffic.barriers, 1);

    // The sink hands the recorded reports back in order, once.
    let sink = composition::Sink::new();
    assert!(sink.is_empty());
    sink.record(report.clone());
    sink.record(report);
    assert_eq!(sink.len(), 2);
    let drained = sink.take();
    assert_eq!(drained.len(), 2);
    assert_eq!(drained[0].frame, 8);
    assert!(sink.take().is_empty());
}

#[test]
fn capture_dependencies_freeze_the_last_prior_writer() {
    let capture_target = Target::Backdrop {
        group: 7,
        region: 0,
    };
    let mut capture = pass(capture_target, true, None, 2);
    capture.capture = Some(crate::render::lower::Capture {
        group: 7,
        region: 0,
        copy_from: Target::Scratch(0),
        resolve: None,
        levels: 1,
    });
    let frame = frame(vec![
        pass(Target::Scratch(0), true, None, 0),
        pass(Target::Scratch(0), false, None, 1),
        capture,
        pass(Target::Scratch(0), false, None, 3),
        pass(Target::Part(0), true, Some(Source::Scratch(0)), 4),
        pass(Target::Scratch(0), true, None, 5),
        pass(Target::Part(1), true, Some(Source::Scratch(0)), 6),
    ]);
    let format = formats(wgpu::TextureFormat::Rgba16Float);
    let native = ExecutionPlan::build(&frame, format, limits(4));
    let portable = ExecutionPlan::portable(&frame, format, Materialization::NoExecutor);
    for plan in [&native, &portable] {
        assert!(plan.loads[0].is_none());
        let continued = plan.loads[1].as_ref().expect("load retains earlier writes");
        assert_eq!(continued.writer, Some(0));
        assert_eq!(continued.image, plan.targets[0]);
        assert_eq!(plan.loads[3].as_ref().expect("later load").writer, Some(1));
        assert!(
            plan.loads[5].is_none(),
            "clear severs the old image dependency"
        );
        let capture = plan.captures[2]
            .as_ref()
            .expect("capture has a read dependency");
        assert_eq!(capture.writer, Some(1));
        assert_eq!(capture.image, plan.targets[0]);
        assert_eq!(capture.kind, ReadKind::FrozenSnapshot);
        let later = plan.reads[4][0].as_ref().expect("read after continuation");
        assert_eq!(later.writer, Some(3));
        assert_eq!(later.image, capture.image);
        let cleared = plan.reads[6][0].as_ref().expect("read after clear");
        assert_eq!(cleared.writer, Some(5));
        assert_ne!(cleared.image, capture.image);
    }
    assert_eq!(native.targets, portable.targets);
    assert_eq!(portable.epochs().count(), 0);
    assert!(portable.is_planned());
    for pass in 0..frame.passes.len() {
        assert_eq!(
            portable.materialization(pass),
            Some(Materialization::NoExecutor)
        );
    }
}

#[test]
fn demand_follows_frozen_writes_and_severs_cleared_generations() {
    let backdrop = Target::Backdrop {
        group: 9,
        region: 0,
    };
    let mut capture = pass(backdrop, true, None, 2);
    capture.capture = Some(crate::render::lower::Capture {
        group: 9,
        region: 0,
        copy_from: Target::Scratch(0),
        resolve: None,
        levels: 1,
    });
    let frame = frame(vec![
        pass(Target::Scratch(0), true, None, 0),
        pass(Target::Scratch(0), false, None, 1),
        capture,
        pass(Target::Scratch(0), false, None, 3),
        pass(
            Target::Part(0),
            true,
            Some(Source::Backdrop {
                group: 9,
                region: 0,
            }),
            4,
        ),
        pass(Target::Scratch(0), true, None, 5),
        pass(Target::Part(1), true, Some(Source::Scratch(0)), 6),
    ]);
    let mut demand = crate::render::composite::demand::Demand::default();
    for plan in [
        ExecutionPlan::portable(
            &frame,
            formats(wgpu::TextureFormat::Rgba16Float),
            Materialization::NoExecutor,
        ),
        ExecutionPlan::build(&frame, formats(wgpu::TextureFormat::Rgba16Float), limits(4)),
    ] {
        demand.update(&frame, &plan, |target, _, region| {
            if target == Target::Part(0) {
                region.add(2, 3, 4, 5);
            }
        });
        for index in [0, 1, 2, 4] {
            assert_eq!(demand.pass(index).pixel_area(), 20, "pass {index}");
        }
        for index in [3, 5, 6] {
            assert!(demand.pass(index).is_empty(), "unneeded pass {index}");
        }
        demand.update(&frame, &plan, |target, _, region| {
            if target == Target::Part(1) {
                region.add(8, 9, 2, 3);
            }
        });
        assert_eq!(demand.pass(5).pixel_area(), 6);
        assert_eq!(demand.pass(6).pixel_area(), 6);
        for index in 0..5 {
            assert!(demand.pass(index).is_empty(), "old generation {index}");
        }
    }
}

#[test]
fn demand_expands_neighborhood_reads_and_tracks_destination_snapshots() {
    let mut frame = frame(vec![
        pass(Target::Part(0), true, None, 0),
        pass(Target::Scratch(0), true, None, 1),
        pass(Target::Part(0), true, Some(Source::Scratch(0)), 2),
    ]);
    frame.passes[2].backdrop_copy = Some([0, 0, 16, 16]);
    frame.instances[2].meta[1] = crate::render::instance::PAINT_BACKDROP;
    let plan = ExecutionPlan::portable(
        &frame,
        formats(wgpu::TextureFormat::Rgba16Float),
        Materialization::NoExecutor,
    );
    assert!(plan.loads[2].is_none());
    assert_eq!(
        plan.backdrops[2]
            .as_ref()
            .expect("destination snapshot")
            .writer,
        Some(0)
    );
    let mut demand = crate::render::composite::demand::Demand::default();
    demand.update(&frame, &plan, |_, _, region| region.add(4, 5, 2, 3));
    assert_eq!(demand.pass(0).pixel_area(), 6);
    assert_eq!(demand.pass(1).pixel_area(), 256);
    assert_eq!(demand.pass(2).pixel_area(), 6);
}
