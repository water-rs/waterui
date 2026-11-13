use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use kurbo::{Affine, Rect, RoundedRect, Vec2};
use rustc_hash::{FxHashMap, FxHashSet};

use cherenkov::testing::LayerOp;
use cherenkov::{
    Animatable, BlendMode, BlendSpace, FilterId, LayerId, Prop, Retarget, ShapeData, SurfaceTree,
};

/// A clip or backdrop change that lands at once.
const fn snapped<T>(target: T) -> Retarget<T> {
    Retarget {
        target,
        animation: None,
        start: None,
    }
}

/// Clips `layer` to `shape` at once.
const fn clip(layer: LayerId, shape: ShapeData) -> LayerOp {
    LayerOp::Clip(layer, Some(snapped(shape)))
}

/// Makes `layer` sample `sample` at once.
const fn backdrop(layer: LayerId, sample: cherenkov::BackdropSample) -> LayerOp {
    LayerOp::Backdrop(layer, Some(snapped(sample)))
}

mod alloc_counter {
    use std::cell::Cell;

    thread_local! {
        // Each initializer is already const; the lint misfires on
        // Android's emulated-TLS expansion (see `render::diag`'s allow).
        #[allow(
            clippy::missing_const_for_thread_local,
            reason = "the initializer is already const; false positive on clippy 1.99"
        )]
        pub(super) static TRACKING: Cell<bool> = const { Cell::new(false) };
        #[allow(
            clippy::missing_const_for_thread_local,
            reason = "the initializer is already const; false positive on clippy 1.99"
        )]
        pub(super) static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
        #[allow(
            clippy::missing_const_for_thread_local,
            reason = "the initializer is already const; false positive on clippy 1.99"
        )]
        pub(super) static REALLOCATIONS: Cell<usize> = const { Cell::new(0) };
    }
}

use alloc_counter::{ALLOCATIONS, REALLOCATIONS, TRACKING};

struct ThreadAllocator;

#[global_allocator]
static ALLOCATOR: ThreadAllocator = ThreadAllocator;

// SAFETY: every method delegates to `System` under identical contracts —
// `layout`/`pointer` are passed through unchanged — and the tracking only
// touches thread-local counters.
unsafe impl GlobalAlloc for ThreadAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` is the caller's allocation request, forwarded.
        let pointer = unsafe { System.alloc(layout) };
        if TRACKING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` is the caller's allocation request, forwarded.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if TRACKING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: `pointer`/`layout` satisfy `System.dealloc` because
        // they came from `System.alloc` — the allocator contract.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // SAFETY: `pointer`/`layout`/`size` are the caller's request,
        // forwarded — the allocator contract.
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if TRACKING.try_with(Cell::get).unwrap_or(false) {
            let _ = REALLOCATIONS.try_with(|count| count.set(count.get() + 1));
        }
        pointer
    }
}

pub(super) fn start_tracking() {
    ALLOCATIONS.with(|count| count.set(0));
    REALLOCATIONS.with(|count| count.set(0));
    TRACKING.with(|tracking| tracking.set(true));
}

pub(super) fn stop_tracking() -> (usize, usize) {
    TRACKING.with(|tracking| tracking.set(false));
    (ALLOCATIONS.with(Cell::get), REALLOCATIONS.with(Cell::get))
}

use super::{
    Candidate, Compositor, Ineligible, Level, Plan, PlanScratch, Source, frames_only, plan,
};
use crate::render::lower::axis_aligned;

/// A compositor that carries axis-aligned transforms and rect or
/// rounded-rect clips, with a budget of two planes.
struct Test;

impl Compositor for Test {
    const BUDGET: usize = 2;
    const HOSTS_OPACITY: bool = true;
    fn expresses_transform(transform: Affine) -> bool {
        axis_aligned(transform)
    }
    fn expresses_clip(clip: &ShapeData) -> bool {
        matches!(clip, ShapeData::Rect(_) | ShapeData::RoundedRect(_))
    }
    fn shows(_: &crate::interop::ExternalFrame) -> bool {
        true
    }
}

const ROOT: LayerId = LayerId::new(0);
const VIDEO: LayerId = LayerId::new(1);
const ABOVE: LayerId = LayerId::new(2);
const BELOW: LayerId = LayerId::new(3);
const PARENT: LayerId = LayerId::new(4);
const SIZE: (u32, u32) = (320, 180);

#[test]
fn pose_plans_reuse_workspace_and_placement_paths() {
    let mut tree = scene();
    let candidates = video();
    let ready = candidates.keys().copied().collect();
    let mut scratch = PlanScratch::default();
    let mut output = Plan::default();
    for _ in 0..3 {
        super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
    }
    let buffers = (
        std::ptr::from_ref(&scratch.order[0]),
        scratch.decisions.as_ptr(),
        output.planes.as_ptr(),
        output.planes[0].path.as_ptr(),
    );
    for x in 0..10 {
        tree.apply(LayerOp::Transform(
            VIDEO,
            prop(Affine::translate((f64::from(x), 0.))),
        ));
        super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
        assert_eq!(output, plan::<Test>(&tree, &candidates, &ready));
        assert_eq!(
            buffers,
            (
                std::ptr::from_ref(&scratch.order[0]),
                scratch.decisions.as_ptr(),
                output.planes.as_ptr(),
                output.planes[0].path.as_ptr()
            )
        );
    }
}

#[test]
fn steady_pose_plan_allocates_nothing() {
    let mut tree = scene();
    let candidates = video();
    let ready = candidates.keys().copied().collect();
    let mut scratch = PlanScratch::default();
    let mut output = Plan::default();
    for _ in 0..4 {
        super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
    }
    for x in 0..8 {
        tree.apply(LayerOp::Transform(
            VIDEO,
            prop(Affine::translate((f64::from(x), 0.))),
        ));
        start_tracking();
        super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
        let counts = stop_tracking();
        assert_eq!(counts, (0, 0), "pose {x}");
    }
    assert_eq!(output, plan::<Test>(&tree, &candidates, &ready));
}

const fn prop<T: Animatable>(target: T) -> Prop<T> {
    Prop {
        target,
        animation: None,
        start: None,
    }
}

/// Root with `BELOW`, `PARENT` (holding `VIDEO`) and `ABOVE`, in that
/// paint order.
fn scene() -> SurfaceTree {
    let mut tree = SurfaceTree::new();
    for id in [VIDEO, ABOVE, BELOW, PARENT] {
        tree.apply(LayerOp::Create(id));
    }
    for (parent, child) in [
        (ROOT, BELOW),
        (ROOT, PARENT),
        (PARENT, VIDEO),
        (ROOT, ABOVE),
    ] {
        tree.apply(LayerOp::Push { parent, child });
    }
    tree
}

fn video() -> FxHashMap<LayerId, super::Candidate> {
    std::iter::once((VIDEO, SIZE.into())).collect()
}

/// The ready set when every candidate's realization is complete.
fn all_ready(candidates: &FxHashMap<LayerId, super::Candidate>) -> FxHashSet<LayerId> {
    candidates.keys().copied().collect()
}

fn verdict(tree: &SurfaceTree) -> Result<Plan, Ineligible> {
    let candidates = video();
    let plan = plan::<Test>(tree, &candidates, &all_ready(&candidates));
    match plan.rejected.as_slice() {
        [] => Ok(plan),
        [(layer, cause)] => {
            assert_eq!(*layer, VIDEO);
            Err(cause.clone())
        }
        more => panic!("one candidate, {} rejections", more.len()),
    }
}

/// An external frame at the surface level with a default blend, no
/// backdrop and an expressible path is promoted, and the surface splits
/// into the part below it and the part above it.
#[test]
fn an_eligible_external_frame_is_promoted_between_two_parts() {
    let plan = verdict(&scene()).expect("eligible");
    assert_eq!(plan.planes.len(), 1);
    let placement = &plan.planes[0];
    assert_eq!(placement.layer, VIDEO);
    assert_eq!(placement.size, SIZE);
    assert_eq!(
        placement.path.iter().map(|l| l.layer).collect::<Vec<_>>(),
        [ROOT, PARENT, VIDEO]
    );
    assert!(plan.trailing, "ABOVE is painted after the video");
    assert_eq!(plan.parts(), 2);
}

/// Only external-frame layers are candidates: an otherwise eligible layer
/// without one stays in the engine.
#[test]
fn only_candidates_are_promoted() {
    let none = plan::<Test>(&scene(), &FxHashMap::default(), &FxHashSet::default());
    assert_eq!(none.planes, []);
    assert_eq!(none.parts(), 1);
    let candidates = video();
    let video = plan::<Test>(&scene(), &candidates, &all_ready(&candidates));
    assert_eq!(
        video.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [VIDEO],
        "eligible layers without an external frame stay in the engine"
    );
}

/// With nothing painted after the plane, no part exists above it.
#[test]
fn no_part_exists_above_a_topmost_plane() {
    let mut tree = scene();
    tree.remove(ABOVE);
    let plan = verdict(&tree).expect("eligible");
    assert!(!plan.trailing);
    assert_eq!(plan.parts(), 1);
}

#[test]
fn an_isolating_ancestor_keeps_the_layer_in_the_engine() {
    let mut tree = scene();
    tree.apply(LayerOp::Opacity(PARENT, prop(0.5)));
    assert_eq!(verdict(&tree), Err(Ineligible::Isolated(PARENT)));
    let mut tree = scene();
    tree.apply(LayerOp::Filter(PARENT, Some(FilterId::new(1))));
    assert_eq!(verdict(&tree), Err(Ineligible::Isolated(PARENT)));
}

#[test]
fn a_filtered_layer_is_not_promoted() {
    let mut tree = scene();
    tree.apply(LayerOp::Filter(VIDEO, Some(FilterId::new(1))));
    assert_eq!(verdict(&tree), Err(Ineligible::Filter));
}

#[test]
fn a_non_default_blend_is_not_promoted() {
    // Under the root, which renders into the surface and never isolates
    // for a blended child; under PARENT the blend would isolate PARENT.
    let mut tree = scene();
    tree.apply(LayerOp::Push {
        parent: ROOT,
        child: VIDEO,
    });
    tree.apply(LayerOp::Blend(VIDEO, BlendMode::Multiply));
    assert_eq!(verdict(&tree), Err(Ineligible::Blend));
    let mut tree = scene();
    tree.apply(LayerOp::Blend(VIDEO, BlendMode::Multiply));
    assert_eq!(verdict(&tree), Err(Ineligible::Isolated(PARENT)));
    // A child blending onto the frame needs the frame's pixels too.
    let mut tree = scene();
    tree.apply(LayerOp::Create(LayerId::new(9)));
    tree.apply(LayerOp::Push {
        parent: VIDEO,
        child: LayerId::new(9),
    });
    tree.apply(LayerOp::Blend(LayerId::new(9), BlendMode::Screen));
    assert_eq!(verdict(&tree), Err(Ineligible::Blend));
}

#[test]
fn a_non_linear_blend_space_is_not_promoted() {
    let mut tree = scene();
    tree.apply(LayerOp::BlendSpace(VIDEO, BlendSpace::SrgbEncoded));
    assert_eq!(verdict(&tree), Err(Ineligible::BlendSpace));
    // An encoded ancestor composites its subtree through an offscreen.
    let mut tree = scene();
    tree.apply(LayerOp::BlendSpace(PARENT, BlendSpace::SrgbEncoded));
    assert_eq!(verdict(&tree), Err(Ineligible::Isolated(PARENT)));
}

#[test]
fn a_surface_level_blend_above_keeps_the_layer_in_the_engine() {
    let mut tree = scene();
    tree.apply(LayerOp::Blend(ABOVE, BlendMode::Multiply));
    assert_eq!(verdict(&tree), Err(Ineligible::BlendAbove(ABOVE)));
    // Below the frame, the blend composites onto the part under it exactly
    // as it would in the engine.
    let mut tree = scene();
    tree.apply(LayerOp::Blend(BELOW, BlendMode::Multiply));
    assert!(verdict(&tree).is_ok());
    // Inside an isolated layer above, the blend stays in its offscreen —
    // the blended child is what isolates `ABOVE` — while `ABOVE` itself
    // composites over the plane, so it must stay known-opaque.
    let mut tree = scene();
    tree.apply(LayerOp::Create(LayerId::new(9)));
    tree.apply(LayerOp::Push {
        parent: ABOVE,
        child: LayerId::new(9),
    });
    tree.apply(LayerOp::Blend(LayerId::new(9), BlendMode::Multiply));
    assert!(verdict(&tree).is_ok());
    // A layer above that is not known to be opaque — here, one below
    // full opacity — makes the platform composite over the plane in its
    // own space, which the engine cannot reproduce.
    tree.apply(LayerOp::Opacity(ABOVE, prop(0.5)));
    assert_eq!(verdict(&tree), Err(Ineligible::TranslucentAbove(ABOVE)));
}

/// The `scene_bar` geometry (#90): the holder's clip ends the video at
/// y = 56 while its content runs past it, and a translucent bar sits at
/// y 58–64 — over the clipped-away part of the footprint only. The clip,
/// not the content rect, decides the overlap.
#[test]
fn a_translucent_layer_beyond_the_clip_does_not_block_promotion() {
    let mut tree = scene();
    tree.apply(LayerOp::Transform(
        PARENT,
        prop(Affine::translate((12.0, 8.0))),
    ));
    tree.apply(clip(
        PARENT,
        ShapeData::RoundedRect(RoundedRect::new(0.0, 0.0, 72.0, 48.0, 6.0)),
    ));
    // Scaled 2×, the 48×32 video's content covers (12, 8)–(108, 72); the
    // holder's clip leaves only (12, 8)–(84, 56) on the plane.
    tree.apply(LayerOp::Transform(VIDEO, prop(Affine::scale(2.0))));
    tree.apply(LayerOp::Opacity(ABOVE, prop(0.5)));
    tree.apply(clip(
        ABOVE,
        ShapeData::Rect(Rect::new(8.0, 58.0, 88.0, 64.0)),
    ));
    let candidates: FxHashMap<_, _> = std::iter::once((VIDEO, (48, 32).into())).collect();
    let plan = plan::<Test>(&tree, &candidates, &all_ready(&candidates));
    assert_eq!(plan.planes.len(), 1, "{:?}", plan.rejected);
}

#[test]
fn a_group_opacity_is_not_promoted_but_a_leaf_opacity_is() {
    let mut tree = scene();
    tree.apply(LayerOp::Opacity(VIDEO, prop(0.5)));
    let plan = verdict(&tree).expect("a childless layer's opacity is a plane property");
    assert!((plan.planes[0].opacity - 0.5).abs() < f32::EPSILON);
    tree.apply(LayerOp::Create(LayerId::new(9)));
    tree.apply(LayerOp::Push {
        parent: VIDEO,
        child: LayerId::new(9),
    });
    assert_eq!(verdict(&tree), Err(Ineligible::GroupOpacity));
}

#[test]
fn an_inexpressible_transform_is_not_promoted() {
    let mut tree = scene();
    tree.apply(LayerOp::Transform(PARENT, prop(Affine::rotate(0.3))));
    assert_eq!(verdict(&tree), Err(Ineligible::Transform(PARENT)));
}

#[test]
fn an_inexpressible_clip_is_not_promoted() {
    let mut tree = scene();
    tree.apply(clip(
        PARENT,
        ShapeData::Circle(kurbo::Circle::new((10.0, 10.0), 5.0)),
    ));
    assert_eq!(verdict(&tree), Err(Ineligible::Clip(PARENT)));
    let mut tree = scene();
    tree.apply(clip(
        VIDEO,
        ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0)),
    ));
    assert!(verdict(&tree).is_ok());
}

/// An ancestor clip pair the engine composites through a clip offscreen
/// (`Lowering::run_clipped`) keeps the layer in the engine — whichever of
/// the pair carries the shape. The candidate's own clip does not count:
/// its scope opens only after the part boundary the candidate starts.
#[test]
fn nested_shaped_clips_are_not_promoted() {
    let rounded = || ShapeData::RoundedRect(RoundedRect::new(0.0, 0.0, 100.0, 50.0, 8.0));
    let mut tree = scene();
    tree.apply(clip(ROOT, rounded()));
    tree.apply(clip(PARENT, rounded()));
    assert_eq!(verdict(&tree), Err(Ineligible::NestedClip(PARENT)));
    // A device-aligned rect under a shaped clip isolates too: only a
    // pair of aligned rects merges in place.
    let mut tree = scene();
    tree.apply(clip(ROOT, rounded()));
    tree.apply(clip(
        PARENT,
        ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0)),
    ));
    assert_eq!(verdict(&tree), Err(Ineligible::NestedClip(PARENT)));
    let mut tree = scene();
    tree.apply(clip(
        ROOT,
        ShapeData::Rect(Rect::new(0.0, 0.0, 200.0, 100.0)),
    ));
    tree.apply(clip(
        PARENT,
        ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0)),
    ));
    assert!(verdict(&tree).is_ok());
    // Under a single shaped ancestor the candidate's own clip is excluded.
    let mut tree = scene();
    tree.apply(clip(PARENT, rounded()));
    tree.apply(clip(VIDEO, rounded()));
    assert!(verdict(&tree).is_ok());
}

#[test]
fn the_budget_goes_to_the_first_candidates_in_paint_order() {
    let tree = scene();
    let candidates: FxHashMap<_, _> = [
        (BELOW, SIZE.into()),
        (VIDEO, SIZE.into()),
        (ABOVE, SIZE.into()),
    ]
    .into_iter()
    .collect();
    let plan = plan::<Test>(&tree, &candidates, &all_ready(&candidates));
    assert_eq!(
        plan.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [BELOW, VIDEO]
    );
    assert_eq!(plan.rejected, [(ABOVE, Ineligible::Budget(Test::BUDGET))]);
    assert!(plan.trailing);
    assert_eq!(plan.parts(), 3);
}

#[test]
fn video_receives_the_budget_before_earlier_recorded_layers() {
    let tree = scene();
    let mut recorded = Candidate::from(SIZE);
    recorded.source = Source::Recorded;
    let candidates: FxHashMap<_, _> = [
        (BELOW, recorded),
        (VIDEO, SIZE.into()),
        (ABOVE, SIZE.into()),
    ]
    .into_iter()
    .collect();
    let plan = plan::<Test>(&tree, &candidates, &all_ready(&candidates));
    assert_eq!(
        plan.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [VIDEO, ABOVE]
    );
    assert_eq!(plan.rejected, [(BELOW, Ineligible::Budget(Test::BUDGET))]);
}

/// The path carries each level's sampled properties, so the content lands
/// where the engine would draw it.
#[test]
fn the_placement_maps_content_to_device_space() {
    let mut tree = scene();
    tree.apply(LayerOp::Transform(
        PARENT,
        prop(Affine::translate((40.0, 30.0))),
    ));
    tree.apply(LayerOp::ScrollOffset(PARENT, prop(Vec2::new(0.0, 10.0))));
    tree.apply(LayerOp::Transform(VIDEO, prop(Affine::scale(0.5))));
    let plan = verdict(&tree).expect("eligible");
    let placement = &plan.planes[0];
    assert_eq!(
        placement.path[1],
        Level {
            layer: PARENT,
            transform: Affine::translate((40.0, 30.0)),
            clip: None,
            scroll: Vec2::new(0.0, 10.0),
        }
    );
    assert_eq!(
        placement.content_to_device(),
        Affine::translate((40.0, 20.0)) * Affine::scale(0.5)
    );
}

/// The plane-only gate (#90): a frame whose only change is new frames on
/// layers the committed plan promotes refreshes through the planes alone
/// — the engine renders nothing for it. A changed layer the plan keeps
/// in the engine, a moved or reshaped promotion, a new candidate, or no
/// installs at all takes the full render path.
#[test]
fn a_plane_only_frame_presents_through_the_planes_alone() {
    let tree = scene();
    let candidates = video();
    let committed = verdict(&tree).expect("eligible");
    let mut scratch = PlanScratch::default();
    let ready = all_ready(&candidates);
    let video_only: FxHashSet<LayerId> = std::iter::once(VIDEO).collect();
    assert!(frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &video_only,
        &mut scratch
    ));
    // A second call on the same scratch — the steady-state video path —
    // agrees.
    assert!(frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &video_only,
        &mut scratch
    ));

    // A new frame on a layer the plan keeps in the engine is a full
    // change: not promoted, it cannot reach a plane.
    let in_engine: FxHashSet<LayerId> = std::iter::once(BELOW).collect();
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &in_engine,
        &mut scratch
    ));
    let mixed: FxHashSet<LayerId> = [VIDEO, BELOW].into_iter().collect();
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &mixed,
        &mut scratch
    ));

    // A tree change the recomputed plan sees — the promotion moved —
    // keeps the full path.
    let mut moved = scene();
    moved.apply(LayerOp::Transform(
        PARENT,
        prop(Affine::translate((8.0, 0.0))),
    ));
    assert_ne!(
        plan::<Test>(&moved, &candidates, &all_ready(&candidates)),
        committed
    );
    assert!(!frames_only::<Test>(
        &committed,
        &moved,
        &candidates,
        &ready,
        &video_only,
        &mut scratch
    ));

    // A differently sized frame on the same layer changes its placement.
    let resized: FxHashMap<_, _> = std::iter::once((VIDEO, (640, 360).into())).collect();
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &resized,
        &all_ready(&resized),
        &video_only,
        &mut scratch
    ));

    // A first frame on another layer makes it a candidate, which changes
    // the plan — here by adding a second promotion.
    let mut two = video();
    two.insert(BELOW, SIZE.into());
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &two,
        &all_ready(&two),
        &mixed,
        &mut scratch
    ));

    // No installs is never a refresh.
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &FxHashSet::default(),
        &mut scratch
    ));
}

#[test]
fn replacing_a_capture_with_a_frame_rebuilds_the_native_stack() {
    let tree = scene();
    let mut candidates = video();
    candidates.get_mut(&VIDEO).expect("candidate").source = Source::Recorded;
    let ready = all_ready(&candidates);
    let committed = plan::<Test>(&tree, &candidates, &ready);
    candidates.get_mut(&VIDEO).expect("candidate").source = Source::Frame;
    let updates = std::iter::once(VIDEO).collect();
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &updates,
        &mut PlanScratch::default()
    ));
}

/// A candidate whose realization is still pending is absent from the
/// committed plan's verdicts — pendingness is not a plan change, so it
/// never blocks the plane-only path; its readiness does (#90).
#[test]
fn a_pending_candidate_does_not_block_a_plane_only_frame() {
    let tree = scene();
    // `BELOW` gained a plane-capable frame but its realization is still
    // pending: a candidate, not yet ready.
    let mut candidates = video();
    candidates.insert(BELOW, SIZE.into());
    let just_video: FxHashSet<LayerId> = std::iter::once(VIDEO).collect();
    let committed = plan::<Test>(&tree, &candidates, &just_video);
    assert_eq!(committed.planes.len(), 1);
    assert_eq!(committed.rejected, []);

    let mut scratch = PlanScratch::default();
    let video_only: FxHashSet<LayerId> = std::iter::once(VIDEO).collect();
    assert!(frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &just_video,
        &video_only,
        &mut scratch
    ));

    // Once `BELOW`'s plane is ready the verdicts change — the frame
    // takes the full path and re-plans.
    let both: FxHashSet<LayerId> = [VIDEO, BELOW].into_iter().collect();
    assert!(!frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &both,
        &video_only,
        &mut scratch
    ));
}

/// A backdrop sampled by the frame itself, or anywhere above it, needs the
/// frame's pixels in the engine; one sampled below it does not.
#[test]
fn a_backdrop_on_or_above_the_layer_keeps_it_in_the_engine() {
    use cherenkov::{Engine, Offscreen, OffscreenFormat};
    let Ok(engine) = Engine::<crate::Gpu>::new(crate::GpuConfig::default()) else {
        return;
    };
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {})
        .expect("offscreen surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let mut tree = scene();
    tree.apply(backdrop(VIDEO, group.sample()));
    assert_eq!(verdict(&tree), Err(Ineligible::Backdrop));
    let mut tree = scene();
    tree.apply(backdrop(ABOVE, group.sample()));
    assert_eq!(verdict(&tree), Err(Ineligible::BackdropAbove(ABOVE)));
    let mut tree = scene();
    tree.apply(backdrop(BELOW, group.sample()));
    assert!(verdict(&tree).is_ok());
}

/// A compositor with the Android container's limits: its hosted layers
/// take only translations and positive axis-aligned scales, rectangular
/// clips and no opacity.
struct Container;

impl Compositor for Container {
    const BUDGET: usize = 1;
    const HOSTS_OPACITY: bool = false;
    fn expresses_transform(transform: Affine) -> bool {
        axis_aligned(transform)
    }
    fn hosts_transform(transform: Affine) -> bool {
        crate::render::surface_control::plan::hosts_transform(transform)
    }
    fn expresses_clip(clip: &ShapeData) -> bool {
        crate::render::surface_control::plan::expresses_clip(clip)
    }
    fn shows(_: &crate::interop::ExternalFrame) -> bool {
        true
    }
}

fn hosted_candidate() -> Candidate {
    Candidate {
        size: SIZE,
        raster: Affine::IDENTITY,
        source: Source::Hosted,
    }
}

/// `VIDEO` as a hosted layer.
fn hosted() -> FxHashMap<LayerId, Candidate> {
    std::iter::once((VIDEO, hosted_candidate())).collect()
}

/// The hosted layer's verdict on compositor `C`: placed, or unplaced with
/// its cause — never kept in the engine. Hosted content is never pending,
/// so the ready set is empty.
fn hosted_verdict<C: Compositor>(tree: &SurfaceTree) -> Result<Plan, Ineligible> {
    let plan = plan::<C>(tree, &hosted(), &FxHashSet::default());
    assert_eq!(
        plan.rejected,
        [],
        "hosted content is never kept in the engine"
    );
    match plan.unplaced.as_slice() {
        [] => Ok(plan),
        [(layer, cause)] => {
            assert_eq!(*layer, VIDEO);
            assert_eq!(plan.planes, [], "an unplaced layer is on no plane");
            Err(cause.clone())
        }
        more => panic!("one candidate, {} unplaced", more.len()),
    }
}

/// A hosted layer sits between the part painted below it and the part
/// painted above it — its children and later siblings — on the path the
/// tree gives it, with no readiness to wait for.
#[test]
fn a_hosted_layer_is_placed_between_the_parts_below_and_above() {
    let plan = hosted_verdict::<Test>(&scene()).expect("eligible");
    assert_eq!(plan.planes.len(), 1);
    let placement = &plan.planes[0];
    assert_eq!((placement.layer, placement.source), (VIDEO, Source::Hosted));
    assert_eq!(
        placement.path.iter().map(|l| l.layer).collect::<Vec<_>>(),
        [ROOT, PARENT, VIDEO]
    );
    assert!(plan.trailing, "ABOVE is painted after the hosted layer");
    assert_eq!(plan.opens_part().collect::<Vec<_>>(), [VIDEO]);
    assert_eq!(plan.parts(), 2);

    // A child of the hosted layer paints above it, in the part above.
    let mut tree = scene();
    tree.remove(ABOVE);
    let plan = hosted_verdict::<Test>(&tree).expect("eligible");
    assert!(!plan.trailing, "nothing is painted after the hosted layer");
    assert_eq!(plan.parts(), 1);
    tree.apply(LayerOp::Create(LayerId::new(9)));
    tree.apply(LayerOp::Push {
        parent: VIDEO,
        child: LayerId::new(9),
    });
    let plan = hosted_verdict::<Test>(&tree).expect("eligible");
    assert!(plan.trailing, "its child is painted after it");
    assert_eq!(plan.parts(), 2);
}

/// Every cause of the mandatory-plane rule leaves the hosted layer
/// unplaced, naming the cause.
#[test]
fn a_hosted_layer_under_an_isolating_ancestor_is_unplaced() {
    let mut tree = scene();
    tree.apply(LayerOp::Opacity(PARENT, prop(0.5)));
    assert_eq!(
        hosted_verdict::<Test>(&tree),
        Err(Ineligible::Isolated(PARENT))
    );
    let mut tree = scene();
    tree.apply(LayerOp::Filter(PARENT, Some(FilterId::new(1))));
    assert_eq!(
        hosted_verdict::<Test>(&tree),
        Err(Ineligible::Isolated(PARENT))
    );
}

#[test]
fn a_filtered_hosted_layer_is_unplaced() {
    let mut tree = scene();
    tree.apply(LayerOp::Filter(VIDEO, Some(FilterId::new(1))));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Filter));
}

#[test]
fn a_blended_hosted_layer_is_unplaced() {
    let mut tree = scene();
    tree.apply(LayerOp::Push {
        parent: ROOT,
        child: VIDEO,
    });
    tree.apply(LayerOp::Blend(VIDEO, BlendMode::Multiply));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Blend));
    // A child blending onto it needs its pixels.
    let mut tree = scene();
    tree.apply(LayerOp::Create(LayerId::new(9)));
    tree.apply(LayerOp::Push {
        parent: VIDEO,
        child: LayerId::new(9),
    });
    tree.apply(LayerOp::Blend(LayerId::new(9), BlendMode::Screen));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Blend));
}

#[test]
fn a_surface_level_blend_above_a_hosted_layer_unplaces_it() {
    let mut tree = scene();
    tree.apply(LayerOp::Blend(ABOVE, BlendMode::Multiply));
    assert_eq!(
        hosted_verdict::<Test>(&tree),
        Err(Ineligible::BlendAbove(ABOVE))
    );
    let mut tree = scene();
    tree.apply(LayerOp::Blend(BELOW, BlendMode::Multiply));
    assert!(hosted_verdict::<Test>(&tree).is_ok());
}

#[test]
fn a_hosted_layer_with_a_group_opacity_is_unplaced() {
    let mut tree = scene();
    tree.apply(LayerOp::Opacity(VIDEO, prop(0.5)));
    let plan = hosted_verdict::<Test>(&tree).expect("a leaf opacity is a plane property");
    assert!((plan.planes[0].opacity - 0.5).abs() < f32::EPSILON);
    tree.apply(LayerOp::Create(LayerId::new(9)));
    tree.apply(LayerOp::Push {
        parent: VIDEO,
        child: LayerId::new(9),
    });
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::GroupOpacity));
}

/// A compositor that cannot fade a hosted container leaves a translucent
/// hosted leaf unplaced; an opaque one is placed.
#[test]
fn a_faded_hosted_layer_is_unplaced_where_the_system_cannot_fade_it() {
    let tree = scene();
    assert!(hosted_verdict::<Container>(&tree).is_ok());
    let mut tree = scene();
    tree.apply(LayerOp::Opacity(VIDEO, prop(0.5)));
    assert_eq!(hosted_verdict::<Container>(&tree), Err(Ineligible::Opacity));
}

#[test]
fn a_hosted_layer_under_an_inexpressible_transform_is_unplaced() {
    let mut tree = scene();
    tree.apply(LayerOp::Transform(PARENT, prop(Affine::rotate(0.3))));
    assert_eq!(
        hosted_verdict::<Test>(&tree),
        Err(Ineligible::Transform(PARENT))
    );
    // A mirror a buffer plane carries is no container transform.
    let mut tree = scene();
    tree.apply(LayerOp::Transform(
        PARENT,
        prop(Affine::scale_non_uniform(-1.0, 1.0)),
    ));
    assert_eq!(
        hosted_verdict::<Container>(&tree),
        Err(Ineligible::Transform(PARENT))
    );
    let mut tree = scene();
    tree.apply(LayerOp::Transform(
        PARENT,
        prop(Affine::translate((4.0, 8.0)) * Affine::scale(2.0)),
    ));
    assert!(hosted_verdict::<Container>(&tree).is_ok());
}

#[test]
fn a_hosted_layer_under_an_inexpressible_clip_is_unplaced() {
    let mut tree = scene();
    tree.apply(clip(
        PARENT,
        ShapeData::Circle(kurbo::Circle::new((10.0, 10.0), 5.0)),
    ));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Clip(PARENT)));
    let mut tree = scene();
    tree.apply(clip(
        PARENT,
        ShapeData::RoundedRect(RoundedRect::new(0.0, 0.0, 100.0, 50.0, 8.0)),
    ));
    assert!(hosted_verdict::<Test>(&tree).is_ok());
    assert_eq!(
        hosted_verdict::<Container>(&tree),
        Err(Ineligible::Clip(PARENT))
    );
}

/// The rules that keep an opportunistic promotion pixel-identical to
/// engine composition do not bind content the engine cannot composite:
/// translucent controls above a hosted layer and nested shaped clips are
/// the system compositor's to draw.
#[test]
fn translucency_above_and_nested_clips_do_not_unplace_a_hosted_layer() {
    let mut tree = scene();
    tree.apply(LayerOp::Opacity(ABOVE, prop(0.5)));
    assert!(hosted_verdict::<Test>(&tree).is_ok());
    let rounded = ShapeData::RoundedRect(RoundedRect::new(0.0, 0.0, 100.0, 50.0, 8.0));
    let mut tree = scene();
    tree.apply(clip(PARENT, rounded.clone()));
    tree.apply(clip(VIDEO, rounded));
    assert!(hosted_verdict::<Test>(&tree).is_ok());
}

/// Hosted layers take the budget before external frames and captures;
/// one beyond the budget is unplaced, while a frame beyond it stays in
/// the engine.
#[test]
fn hosted_layers_take_the_budget_first_and_fail_beyond_it() {
    let tree = scene();
    let candidates: FxHashMap<_, _> = [
        (BELOW, Candidate::from(SIZE)),
        (VIDEO, hosted_candidate()),
        (ABOVE, hosted_candidate()),
    ]
    .into_iter()
    .collect();
    let ready = all_ready(&candidates);
    let first = plan::<Test>(&tree, &candidates, &ready);
    assert_eq!(
        first.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [VIDEO, ABOVE]
    );
    assert_eq!(first.rejected, [(BELOW, Ineligible::Budget(Test::BUDGET))]);
    assert_eq!(first.unplaced, []);

    let candidates: FxHashMap<_, _> = [
        (BELOW, hosted_candidate()),
        (VIDEO, hosted_candidate()),
        (ABOVE, hosted_candidate()),
    ]
    .into_iter()
    .collect();
    let plan = plan::<Test>(&tree, &candidates, &FxHashSet::default());
    assert_eq!(
        plan.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [BELOW, VIDEO]
    );
    assert_eq!(plan.rejected, []);
    assert_eq!(plan.unplaced, [(ABOVE, Ineligible::Budget(Test::BUDGET))]);
}

/// A geometry change on the hosted layer's path moves its placement and
/// nothing else: the plane keeps its layer, source, extent and slot in
/// the stack, which is what lets a realization keep its native nodes.
#[test]
fn moving_a_hosted_layer_keeps_its_plane() {
    let committed = hosted_verdict::<Test>(&scene()).expect("eligible");
    let mut tree = scene();
    tree.apply(LayerOp::Transform(
        PARENT,
        prop(Affine::translate((40.0, 30.0))),
    ));
    tree.apply(LayerOp::ScrollOffset(PARENT, prop(Vec2::new(0.0, 10.0))));
    tree.apply(clip(
        VIDEO,
        ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0)),
    ));
    let moved = hosted_verdict::<Test>(&tree).expect("eligible");
    assert_ne!(moved, committed);
    let identity = |plan: &Plan| {
        plan.planes
            .iter()
            .map(|p| (p.layer, p.source, p.size, p.path.len()))
            .collect::<Vec<_>>()
    };
    assert_eq!(identity(&moved), identity(&committed));
    assert_eq!(moved.trailing, committed.trailing);
    assert_eq!(
        moved.planes[0].content_to_device(),
        Affine::translate((40.0, 20.0))
    );
}

/// A committed plan whose hosted layer became unplaceable is never the
/// plane-only frame's plan: the frame takes the full path, which fails.
#[test]
fn an_unplaceable_hosted_layer_never_refreshes_through_the_planes() {
    let tree = scene();
    let candidates: FxHashMap<_, _> = [(VIDEO, hosted_candidate()), (BELOW, SIZE.into())]
        .into_iter()
        .collect();
    let ready = all_ready(&candidates);
    let committed = plan::<Test>(&tree, &candidates, &ready);
    assert_eq!(committed.planes.len(), 2);
    let frames: FxHashSet<LayerId> = std::iter::once(BELOW).collect();
    let mut scratch = PlanScratch::default();
    assert!(frames_only::<Test>(
        &committed,
        &tree,
        &candidates,
        &ready,
        &frames,
        &mut scratch
    ));
    let mut filtered = scene();
    filtered.apply(LayerOp::Filter(VIDEO, Some(FilterId::new(1))));
    assert!(!frames_only::<Test>(
        &committed,
        &filtered,
        &candidates,
        &ready,
        &frames,
        &mut scratch
    ));
    let failed = plan::<Test>(&filtered, &candidates, &ready);
    assert_eq!(failed.unplaced, [(VIDEO, Ineligible::Filter)]);
    assert!(!frames_only::<Test>(
        &failed,
        &filtered,
        &candidates,
        &ready,
        &frames,
        &mut scratch
    ));
}

/// A hosted layer under or above a backdrop sample needs pixels the
/// engine never has; a backdrop sampled below it does not.
#[test]
fn a_backdrop_on_or_above_a_hosted_layer_unplaces_it() {
    use cherenkov::{Engine, Offscreen, OffscreenFormat};
    let Ok(engine) = Engine::<crate::Gpu>::new(crate::GpuConfig::default()) else {
        return;
    };
    let surface = engine
        .surface(Offscreen::new((8, 8), OffscreenFormat::LinearF16), || {})
        .expect("offscreen surface");
    let group = surface.backdrop_group_unfiltered(cherenkov::CaptureScale::FULL);
    let mut tree = scene();
    tree.apply(backdrop(VIDEO, group.sample()));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Backdrop));
    let mut tree = scene();
    tree.apply(backdrop(ABOVE, group.sample()));
    assert_eq!(
        hosted_verdict::<Test>(&tree),
        Err(Ineligible::BackdropAbove(ABOVE))
    );
    let mut tree = scene();
    tree.apply(backdrop(BELOW, group.sample()));
    assert!(hosted_verdict::<Test>(&tree).is_ok());
}

/// The first hosted layer in paint order on a surface with no planes is
/// the one its render names.
#[test]
fn the_first_hosted_layer_in_paint_order_is_found() {
    let tree = scene();
    assert_eq!(
        super::first_in_paint_order(&tree, |layer| [ABOVE, VIDEO].contains(&layer)),
        Some(VIDEO)
    );
    assert_eq!(super::first_in_paint_order(&tree, |_| false), None);
}

/// A compositor whose regions above a host-window mark present as
/// engine-composited bands, as Android's does, with a budget of `N`
/// planes.
struct Bands<const N: usize = 4>;

impl<const N: usize> Compositor for Bands<N> {
    const BUDGET: usize = N;
    const HOSTS_OPACITY: bool = true;
    const HOST_WINDOW_BANDS: bool = true;
    fn expresses_transform(transform: Affine) -> bool {
        axis_aligned(transform)
    }
    fn expresses_clip(clip: &ShapeData) -> bool {
        matches!(clip, ShapeData::Rect(_) | ShapeData::RoundedRect(_))
    }
    fn shows(_: &crate::interop::ExternalFrame) -> bool {
        true
    }
}

fn host_window() -> Candidate {
    Candidate {
        size: (0, 0),
        raster: Affine::IDENTITY,
        source: Source::HostWindow,
    }
}

/// With band regions, `BELOW` as a host window's mark puts the video in
/// the band above it: the frame composites into that band, a hosted
/// layer there is unplaced, and the mark alone splits the surface.
/// Painted beneath the mark, the video still promotes.
#[test]
fn nothing_above_a_host_window_mark_reaches_a_plane() {
    let tree = scene();
    let frame: FxHashMap<LayerId, Candidate> = [(BELOW, host_window()), (VIDEO, SIZE.into())]
        .into_iter()
        .collect();
    let banded = plan::<Bands>(&tree, &frame, &all_ready(&frame));
    assert_eq!(
        banded.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [BELOW]
    );
    assert_eq!(
        banded.rejected,
        [(VIDEO, Ineligible::AboveHostWindow(BELOW))]
    );
    assert_eq!(
        banded.parts(),
        2,
        "the base part and the band region above the mark"
    );
    // The same tree on a compositor without bands promotes both.
    let unbanded = plan::<Test>(&tree, &frame, &all_ready(&frame));
    assert_eq!(
        unbanded.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [BELOW, VIDEO]
    );

    let hosted: FxHashMap<LayerId, Candidate> =
        [(BELOW, host_window()), (VIDEO, hosted_candidate())]
            .into_iter()
            .collect();
    let hosted_plan = plan::<Bands>(&tree, &hosted, &FxHashSet::default());
    assert_eq!(
        hosted_plan.unplaced,
        [(VIDEO, Ineligible::AboveHostWindow(BELOW))]
    );

    let above: FxHashMap<LayerId, Candidate> = [(ABOVE, host_window()), (VIDEO, SIZE.into())]
        .into_iter()
        .collect();
    let beneath = plan::<Bands>(&tree, &above, &all_ready(&above));
    assert_eq!(
        beneath.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [VIDEO, ABOVE]
    );
    assert!(
        !beneath.trailing,
        "nothing is painted above the topmost mark"
    );
}

/// A mark under a clip that encloses nothing opens no region, while its
/// host still orders the window: the region above the next mark is named
/// by that mark's own window id, never by its position among the planned
/// marks, so its band is the one the host stacks above that window.
#[test]
fn a_band_region_is_named_by_its_marks_window() {
    const HIDDEN: u64 = 7;
    const SHOWN: u64 = 9;
    let mut tree = scene();
    tree.apply(LayerOp::HostWindow(VIDEO, Some(HIDDEN)));
    tree.apply(LayerOp::HostWindow(BELOW, Some(SHOWN)));
    tree.apply(clip(
        PARENT,
        ShapeData::Circle(kurbo::Circle::new((10.0, 10.0), 0.0)),
    ));
    let marks: FxHashMap<LayerId, Candidate> = [(VIDEO, host_window()), (BELOW, host_window())]
        .into_iter()
        .collect();
    let banded = plan::<Bands>(&tree, &marks, &FxHashSet::default());
    let windows: Vec<_> = banded
        .planes
        .iter()
        .map(|p| tree.layer(p.layer).host_window())
        .collect();
    assert_eq!(
        windows,
        [Some(SHOWN)],
        "the clipped-away mark opens no region"
    );
    assert_eq!(
        banded.parts(),
        2,
        "the base part and the region above the shown mark"
    );
    // Unclipped, both marks open regions, each named by its own window.
    tree.apply(LayerOp::Clip(PARENT, None));
    let both = plan::<Bands>(&tree, &marks, &FxHashSet::default());
    let windows: Vec<_> = both
        .planes
        .iter()
        .map(|p| tree.layer(p.layer).host_window())
        .collect();
    assert_eq!(windows, [Some(SHOWN), Some(HIDDEN)]);
}

/// Two platform views stacked with interactive content between and above
/// them, on a surface with one plane: the marks only split the paint
/// order, so neither they nor their bands spend the budget, and the video
/// beneath the first mark still promotes (water-rs/waterui#2225).
#[test]
fn host_window_marks_take_no_plane_budget() {
    const FIRST: LayerId = LayerId::new(5);
    const CARD: LayerId = LayerId::new(6);
    const SECOND: LayerId = LayerId::new(7);
    const OVERLAY: LayerId = LayerId::new(8);
    let mut tree = SurfaceTree::new();
    for id in [VIDEO, FIRST, CARD, SECOND, OVERLAY] {
        tree.apply(LayerOp::Create(id));
        tree.apply(LayerOp::Push {
            parent: ROOT,
            child: id,
        });
    }
    tree.apply(LayerOp::HostWindow(FIRST, Some(1)));
    tree.apply(LayerOp::HostWindow(SECOND, Some(2)));
    let candidates: FxHashMap<LayerId, Candidate> = [
        (VIDEO, SIZE.into()),
        (FIRST, host_window()),
        (SECOND, host_window()),
    ]
    .into_iter()
    .collect();
    let banded = plan::<Bands<1>>(&tree, &candidates, &all_ready(&candidates));
    assert_eq!(banded.unplaced, []);
    assert_eq!(banded.rejected, []);
    assert_eq!(
        banded.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [VIDEO, FIRST, SECOND]
    );
    assert_eq!(
        banded.parts(),
        4,
        "the parts beneath and above the video, then each mark's band"
    );
}

/// A scroll view's inner layer as the Android renderer lowers it while the
/// overscroll stretch shows — clipped to the viewport, offset by its scroll
/// position and filtered with `filter` — holding two host-window marks with
/// content between and above them.
fn stretched_scroll(filter: FilterId) -> (SurfaceTree, FxHashMap<LayerId, Candidate>) {
    const INNER: LayerId = LayerId::new(10);
    const FIRST: LayerId = LayerId::new(11);
    const CARD: LayerId = LayerId::new(12);
    const SECOND: LayerId = LayerId::new(13);
    const TAIL: LayerId = LayerId::new(15);
    let mut tree = SurfaceTree::new();
    for id in [INNER, FIRST, CARD, SECOND, TAIL] {
        tree.apply(LayerOp::Create(id));
    }
    for (parent, child) in [
        (ROOT, INNER),
        (INNER, FIRST),
        (INNER, CARD),
        (INNER, SECOND),
        (INNER, TAIL),
    ] {
        tree.apply(LayerOp::Push { parent, child });
    }
    tree.apply(clip(
        INNER,
        ShapeData::Rect(Rect::new(0.0, 0.0, 200.0, 300.0)),
    ));
    tree.apply(LayerOp::ScrollOffset(INNER, prop(Vec2::new(0.0, 40.0))));
    tree.apply(LayerOp::Filter(INNER, Some(filter)));
    tree.apply(LayerOp::HostWindow(FIRST, Some(1)));
    tree.apply(LayerOp::HostWindow(SECOND, Some(2)));
    let marks = [(FIRST, host_window()), (SECOND, host_window())]
        .into_iter()
        .collect();
    (tree, marks)
}

const STRETCH: LayerId = LayerId::new(10);

/// The overscroll stretch is a pure coordinate remap, so a mark inside the
/// stretched inner layer is placed: the lowering renders the layer once for
/// the base and once per band, each under the same device-space stretch.
/// Before, the filter made the layer an isolating ancestor and the mark
/// failed with `Isolated`.
#[test]
fn a_mark_under_a_coordinate_remap_filter_is_placed() {
    let (tree, marks) = stretched_scroll(FilterId::new(1).with_coordinate_remap(true));
    let plan = plan::<Bands>(&tree, &marks, &FxHashSet::default());
    assert_eq!(plan.unplaced, []);
    assert_eq!(plan.rejected, []);
    assert_eq!(
        plan.planes.iter().map(|p| p.layer).collect::<Vec<_>>(),
        [LayerId::new(11), LayerId::new(13)]
    );
    // Both marks keep the inner layer's clip and scroll on their path, so
    // the host places each platform view in the scrolled viewport.
    for plane in &plan.planes {
        let inner = &plane.path[1];
        assert_eq!(inner.layer, STRETCH);
        assert_eq!(inner.scroll, Vec2::new(0.0, 40.0));
        assert!(inner.clip.is_some());
    }
    assert_eq!(
        plan.parts(),
        3,
        "the base part beneath the first mark and one band per mark"
    );
}

/// A filter that is not a coordinate remap changes the pixels around it,
/// so it still keeps the mark from the surface level.
#[test]
fn a_mark_under_a_filter_that_is_not_a_remap_stays_unplaced() {
    let (tree, marks) = stretched_scroll(FilterId::new(1));
    let plan = plan::<Bands>(&tree, &marks, &FxHashSet::default());
    assert_eq!(
        plan.unplaced,
        [
            (LayerId::new(11), Ineligible::Isolated(STRETCH)),
            (LayerId::new(13), Ineligible::Isolated(STRETCH)),
        ]
    );
    assert_eq!(plan.planes, []);
}

/// Splitting needs the group to composite as the plain filtered whole: a
/// group opacity, a blend or a projective pose on the filtered layer still
/// fail, and so does a compositor without bands, whose marks are planes.
#[test]
fn a_remap_filter_splits_only_a_plain_group_on_a_band_compositor() {
    let remap = FilterId::new(1).with_coordinate_remap(true);
    for style in [
        LayerOp::Opacity(STRETCH, prop(0.5)),
        LayerOp::Blend(STRETCH, BlendMode::Multiply),
    ] {
        let (mut tree, marks) = stretched_scroll(remap);
        tree.apply(style);
        let plan = plan::<Bands>(&tree, &marks, &FxHashSet::default());
        assert_eq!(plan.planes, []);
        assert_eq!(plan.unplaced.len(), 2);
        assert!(
            plan.unplaced
                .iter()
                .all(|(_, cause)| *cause == Ineligible::Isolated(STRETCH))
        );
    }
    let (tree, marks) = stretched_scroll(remap);
    let plan = plan::<Test>(&tree, &marks, &FxHashSet::default());
    assert_eq!(plan.planes, []);
    assert_eq!(plan.unplaced.len(), 2);
}

/// A child blending within the stretched group would read only its own
/// region's content once the group splits, so the group is not split.
#[test]
fn a_blended_child_keeps_the_stretched_group_whole() {
    const GLOW: LayerId = LayerId::new(14);
    let (mut tree, marks) = stretched_scroll(FilterId::new(1).with_coordinate_remap(true));
    tree.apply(LayerOp::Create(GLOW));
    tree.apply(LayerOp::Push {
        parent: STRETCH,
        child: GLOW,
    });
    tree.apply(LayerOp::Blend(GLOW, BlendMode::Multiply));
    let plan = plan::<Bands>(&tree, &marks, &FxHashSet::default());
    assert_eq!(plan.planes, []);
    assert_eq!(plan.unplaced.len(), 2);
    assert!(
        plan.unplaced
            .iter()
            .all(|(_, cause)| *cause == Ineligible::Isolated(STRETCH))
    );
}
