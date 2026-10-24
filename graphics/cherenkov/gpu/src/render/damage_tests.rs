use super::*;
use crate::render::{
    instance::{KIND_SPAN, Shape, affine},
    lower::{DrawRange, Pass, PipelineKind, ShaderVariant},
};

fn frame(items: &[(usize, f32)]) -> Frame {
    let mut frame = Frame::default();
    let layer = cherenkov::SurfaceTree::new().root();
    for &(op, x) in items {
        frame.command_sources.push(CommandSource {
            commands: frame.commands.len()..frame.commands.len() + 1,
            first_op: op,
            root: Target::Part(0),
            layer,
            mask_epoch: None,
        });
        let start = frame.instances.len();
        let mut instance = Instance::new(KIND_SPAN);
        instance.bounds = [x, 10.0, x + 10.0, 20.0];
        instance.color = [1.0; 4];
        frame.instances.push(instance);
        frame.commands.push(Command {
            instances: start..start + 1,
            revision: 1,
        });
    }
    frame.passes.push(Pass {
        target: Target::Part(0),
        clear: Some([0.0; 4]),
        ranges: vec![DrawRange {
            source: None,
            image: None,
            mask: None,
            pipeline: PipelineKind::SrcOver,
            variant: ShaderVariant::Simple,
            instances: 0..u32::try_from(items.len()).expect("small test scene"),
            gpu: None,
        }],
        space: cherenkov::BlendSpace::Linear,
        region: [0, 0, 100, 100],
        backdrop_copy: None,
        capture: None,
        tier_ranges: [Range::default(), Range::default()],
    });
    frame
}

fn contains(commands: &Commands, x: u32, y: u32) -> bool {
    commands.regions[0]
        .rectangles()
        .iter()
        .any(|r| (r.x0()..r.x1()).contains(&x) && (r.y0()..r.y1()).contains(&y))
}

#[test]
fn movement_and_removal_keep_last_submitted_footprints() {
    let mut history = Commands::default();
    history.update(&frame(&[(0, 10.0)]));
    history.submitted();
    history.update(&frame(&[(0, 10.0)]));
    assert_eq!(history.pixels(), 0);
    history.update(&frame(&[(0, 40.0)]));
    assert_eq!(history.pixels(), 288);
    assert!(contains(&history, 10, 10) && contains(&history, 40, 10));
    // A preparation that never reached the queue is not the old picture.
    history.update(&frame(&[(0, 60.0)]));
    assert!(contains(&history, 10, 10) && contains(&history, 60, 10));
    assert!(!contains(&history, 40, 10));
    history.submitted();
    history.submitted();
    history.update(&frame(&[]));
    assert_eq!(history.pixels(), 144);
    assert!(contains(&history, 60, 10));
    history.submitted();
    history.update(&frame(&[]));
    assert_eq!(history.pixels(), 0);
    history.forget();
    history.update(&frame(&[]));
    assert!(
        history.pass(0).is_full(),
        "replacement attachment must initialize even an empty scene"
    );
}

#[test]
fn insertion_preserves_survivors_but_reordering_invalidates_overlap() {
    let mut history = Commands::default();
    history.update(&frame(&[(1, 10.0), (2, 40.0)]));
    history.submitted();
    history.update(&frame(&[(0, 70.0), (1, 10.0), (2, 40.0)]));
    assert_eq!(history.pixels(), 144);
    assert!(contains(&history, 70, 10));
    assert!(!contains(&history, 10, 10));
    history.submitted();
    history.update(&frame(&[(0, 70.0), (2, 40.0), (1, 10.0)]));
    assert!(history.regions[0].is_full());
}

#[test]
fn same_bounds_clipping_and_replaced_geometry_are_changes() {
    let mut scene = frame(&[(0, 10.0)]);
    let mut history = Commands::default();
    scene.instances[0].meta[3] |= FLAG_HAS_CLIP << 24;
    scene.instances[0].clip = Shape::rect([5.0, 5.0]);
    scene.instances[0].clip_inv = affine(Affine::translate((-15.0, -15.0)));
    history.update(&scene);
    history.submitted();
    scene.instances[0].meta[2] = 17; // rebased gradient allocation alone changes no paint
    history.update(&scene);
    assert_eq!(history.pixels(), 0);
    scene.instances[0].clip.radii = [3.0; 4];
    scene.instances[0].clip.exponent = 2.0;
    history.update(&scene);
    assert_eq!(history.pixels(), 144);
    history.submitted();
    scene.commands[0].revision += 1; // new path can have the same outer bounds
    history.update(&scene);
    assert_eq!(history.pixels(), 144);
}

#[test]
fn attachment_and_synthetic_changes_keep_the_submitted_baseline() {
    let mut scene = frame(&[(0, 10.0)]);
    let mut history = Commands::default();
    history.update(&scene);
    history.submitted();
    scene.passes[0].clear = Some([0.5; 4]);
    history.update(&scene);
    assert!(history.pass(0).is_full(), "background color changed");
    scene.passes[0].clear = Some([0.0; 4]);
    history.update(&scene);
    assert!(
        history.pass(0).is_empty(),
        "abandoned clear never became the baseline"
    );
    scene.commands.clear(); // this draw is now a synthetic composite
    scene.command_sources.clear();
    history.update(&scene);
    history.submitted();
    scene.instances[0].color[3] = 0.5;
    history.update(&scene);
    assert!(history.pass(0).is_full(), "composite opacity changed");
    history.submitted();
    history.update(&scene);
    assert!(history.pass(0).is_empty());
    scene.shadows.push((
        0,
        crate::render::shadow::Parameters {
            transform: Affine::IDENTITY,
            sigma: 2.0,
            spread: 1.0,
        },
    ));
    history.update(&scene);
    assert!(history.pass(0).is_full(), "shadow added");
    history.submitted();
    scene.shadows[0].1.sigma = 3.0;
    history.update(&scene);
    assert!(history.pass(0).is_full(), "shadow sampling radius changed");
}

#[test]
fn resource_versions_and_animated_ranges_are_local() {
    let scene = frame(&[(0, 10.0), (1, 70.0)]);
    let mut history = Commands::default();
    history.update_resources(&scene, |_| 1);
    history.submitted();
    history.update_resources(&scene, |_| 1);
    assert!(history.pass(0).is_empty());
    history.update_resources(&scene, |_| 2);
    assert_eq!(
        history.pixels(),
        288,
        "new external frame updates its two commands"
    );
    history.submitted();
    history.update_resources(&scene, |_| 2);
    let mut range = scene.passes[0].ranges[0].clone();
    range.instances = 0..1;
    history.invalidate_range(&scene, 0, &range);
    assert_eq!(
        history.pixels(),
        144,
        "animated paint updates only its draw"
    );
    history.invalidate_pass(&scene, 0);
    assert!(
        history.pass(0).is_full(),
        "filter can affect its entire output"
    );
}
