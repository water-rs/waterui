//! Wrapper opacity, offset, rotation and scale are engine-run motions
//! (water-rs/waterui#2203): the layer receives each change as a `Prop`
//! carrying the view's animation and its start on Hydrolysis's frame clock,
//! the engine samples it, and Hydrolysis evaluates the same track at event
//! time for hit testing. These tests drive the mirror's `SurfaceTree` —
//! the sampler the GPU backend runs — and compare what it samples with
//! Hydrolysis's own evaluation at the same instants.

use super::*;
use crate::renderer::mount::motion::{LayerMotion, Motion};
use crate::time::Instant;
use cherenkov::LayerId;
use waterui::animation::Animation;
use waterui_graphics::animation::engine_animation;

/// The instants after the change at which both sides are compared, in
/// milliseconds: the start itself, mid-curve and mid-spring frames, each
/// curve's end, and a frame long after every spring settled.
const SAMPLES_MS: [u64; 14] = [
    0, 8, 16, 33, 50, 100, 150, 200, 250, 300, 400, 600, 900, 3_000,
];

/// The wrapper motions a mounted tree's frame layers follow.
struct Wrappers {
    opacity: Vec<(LayerId, Motion<f32>)>,
    transforms: Vec<(
        LayerId,
        Rc<LayerMotion>,
        Rc<crate::renderer::mount::Placement>,
    )>,
}

fn wrappers(renderer: &HydrolysisRenderer<ParleyEngine>) -> Wrappers {
    let mut found = Wrappers {
        opacity: Vec::new(),
        transforms: Vec::new(),
    };
    let mut pending = vec![Rc::clone(&renderer.core.root_core.cell)];
    while let Some(cell) = pending.pop() {
        cell.children(&mut pending);
        let Some(retained) = cell.try_retained() else {
            continue;
        };
        let Some(frame) = retained
            .layers
            .borrow()
            .as_ref()
            .map(|layers| layers.frame().id())
        else {
            continue;
        };
        let lowered = retained.lowered.borrow();
        let Some(program) = lowered.as_ref() else {
            continue;
        };
        if let Some(motion) = &program.opacity_motion {
            found.opacity.push((frame, motion.clone()));
        }
        if let Some(motion) = &program.motion {
            found
                .transforms
                .push((frame, Rc::clone(motion), Rc::clone(cell.placement())));
        }
    }
    found
}

/// One curve value through the shared evaluator, from `from` toward
/// `target` `elapsed` into `animation`: the framework's own evaluation of
/// the change, independent of the motion's bookkeeping.
fn evaluated(animation: &Animation, from: f64, target: f64, elapsed: Duration) -> f64 {
    let timing = engine_animation(animation)
        .timing()
        .expect("a curve or spring animation has a timing");
    let (mut position, mut velocity) = ([0.0], [0.0]);
    timing.sample_lanes(
        &[from],
        &[0.0],
        &[target],
        elapsed.as_secs_f64(),
        &mut position,
        &mut velocity,
    );
    position[0]
}

/// Every wrapper kind under its own animation — the scale's axes under
/// different ones — changes at one frame-clock instant; at every sampled
/// instant the engine's layer values equal Hydrolysis's evaluation exactly.
#[test]
#[allow(
    clippy::float_cmp,
    reason = "both sides run the same evaluator on the same track at the same instant; any difference is a parity defect, not rounding"
)]
fn the_engine_samples_the_tracks_hydrolysis_evaluates() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 320.0, 320.0);
    let phase = Binding::container(false);
    let fade = Animation::ease_in_out(Duration::from_millis(300));
    let stretch = Animation::linear(Duration::from_millis(200));
    let squash = Animation::spring(300.0, 20.0);
    let turn = Animation::ease_in_out(Duration::from_millis(250));
    let slide = Animation::linear(Duration::from_millis(150));
    let lift = Animation::spring(180.0, 14.0);
    let driven = |rest: f32, moved: f32, animation: &Animation| {
        phase
            .clone()
            .map(move |on| if on { moved } else { rest })
            .with(animation.clone())
    };
    let view = vstack((button("Probe")
        .action(|| {})
        .rotation(driven(0.0, 30.0, &turn))
        .scale(driven(1.0, 1.5, &stretch), driven(1.0, 0.5, &squash))
        .offset(driven(0.0, 40.0, &slide), driven(0.0, -20.0, &lift))
        .opacity(driven(1.0, 0.4, &fade)),));

    capture_root_window(&mut renderer, view, &env, bounds);
    renderer.commit_mirror();
    let found = wrappers(&renderer);
    assert_eq!(found.opacity.len(), 1, "one opacity wrapper");
    assert_eq!(
        found.transforms.len(),
        3,
        "rotation, scale and offset wrappers"
    );

    let start: Instant = renderer.frame_instant();
    phase.set(true);
    renderer.commit_mirror();

    for ms in SAMPLES_MS {
        let at = start + Duration::from_millis(ms);
        renderer.set_frame_instant(at);
        renderer.mirror().sample(at);
        // An event at `at`: placement evaluates every motion there.
        let _ = renderer.registries();

        for (layer, motion) in &found.opacity {
            let sampled = renderer.mirror().layer(*layer).opacity;
            assert_eq!(sampled, motion.value_at(at), "opacity at {ms}ms");
            #[expect(
                clippy::cast_possible_truncation,
                reason = "the opacity's lane is the f32 value widened; narrowing it back is exact"
            )]
            let expected = evaluated(&fade, 1.0, f64::from(0.4_f32), at - start) as f32;
            let expected = if ms >= 300 { 0.4 } else { expected };
            assert_eq!(sampled, expected, "opacity at {ms}ms against the evaluator");
        }
        for (layer, motion, placement) in &found.transforms {
            let sampled = renderer.mirror().layer(*layer).transform;
            assert_eq!(
                sampled,
                motion.matrix(placement.transform(), Some(at)),
                "the frame layer's transform at {ms}ms"
            );
        }
    }

    // The scale's axes ran their own animations from the one start: the
    // curve is done at 200ms while the spring still moves.
    let scale = found
        .transforms
        .iter()
        .find_map(|(_, motion, _)| motion.scale_axes()[0].clone().map(|x| (motion, x)))
        .expect("the scale wrapper");
    let mid = start + Duration::from_millis(250);
    assert_eq!(scale.1.value_at(mid), 1.5, "the x curve is done at 250ms");
    let y = scale.0.scale_axes()[1]
        .as_ref()
        .expect("the scale's y axis")
        .value_at(mid);
    assert_eq!(
        y,
        evaluated(&squash, 1.0, 0.5, mid - start),
        "the y spring is mid-flight at 250ms"
    );
    assert_ne!(y, 0.5, "the y spring has not settled at 250ms");
}

/// Hit testing places the target where the engine draws it: the probe's
/// window-space target equals its content bounds under the engine's
/// sampled layer chain, at instants mid-motion and at rest.
#[test]
fn hit_testing_places_the_target_where_the_engine_draws_it() {
    let mut renderer = test_renderer();
    let env = test_environment();
    let bounds = Rect::new(0.0, 0.0, 320.0, 320.0);
    let phase = Binding::container(false);
    let driven = |rest: f32, moved: f32, animation: Animation| {
        phase
            .clone()
            .map(move |on| if on { moved } else { rest })
            .with(animation)
    };
    let view = vstack((button("Probe")
        .action(|| {})
        .scale(
            driven(1.0, 1.25, Animation::linear(Duration::from_millis(200))),
            driven(1.0, 0.75, Animation::spring(300.0, 20.0)),
        )
        .offset(
            driven(
                0.0,
                40.0,
                Animation::ease_in_out(Duration::from_millis(150)),
            ),
            driven(0.0, 24.0, Animation::spring(180.0, 14.0)),
        ),));

    capture_root_window(&mut renderer, view, &env, bounds);
    renderer.commit_mirror();
    let probe = |renderer: &mut HydrolysisRenderer<ParleyEngine>| {
        let targets = renderer
            .registries()
            .pointer_targets
            .iter()
            .filter(|target| target.interaction.is_some())
            .map(|target| target.bounds)
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 1, "the probe registers one target");
        targets[0]
    };
    let rest = probe(&mut renderer);
    // The innermost wrapper frame: its world transform carries the whole
    // motion chain the probe draws under.
    let (inner, scale_motion, placement) = wrappers(&renderer)
        .transforms
        .into_iter()
        .find(|(_, motion, _)| motion.scale_axes()[0].is_some())
        .expect("the scale wrapper");
    let local = renderer
        .mirror()
        .world(inner)
        .inverse()
        .transform_rect_bbox(rest);

    let start: Instant = renderer.frame_instant();
    phase.set(true);
    renderer.commit_mirror();
    for ms in SAMPLES_MS {
        let at = start + Duration::from_millis(ms);
        renderer.set_frame_instant(at);
        renderer.mirror().sample(at);
        let hit = probe(&mut renderer);
        let drawn = renderer.mirror().world(inner).transform_rect_bbox(local);
        for (hit, drawn) in [
            (hit.x0, drawn.x0),
            (hit.y0, drawn.y0),
            (hit.x1, drawn.x1),
            (hit.y1, drawn.y1),
        ] {
            approx::assert_relative_eq!(hit, drawn, epsilon = 1e-9);
        }
        assert_eq!(
            renderer.mirror().layer(inner).transform,
            scale_motion.matrix(placement.transform(), Some(at)),
            "the scale frame's transform at {ms}ms"
        );
    }
    let settled = probe(&mut renderer);
    assert!(
        settled.x0 > rest.x0 && settled.y0 > rest.y0,
        "the probe rests moved: {rest:?} -> {settled:?}"
    );
}
