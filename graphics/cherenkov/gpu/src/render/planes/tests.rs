use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use kurbo::{Affine, Rect, RoundedRect, Vec2};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::render::glyph::Atlas;
use crate::render::lower::{ContentData, Frame, GlyphContext, Lowering};
use cherenkov::testing::LayerOp;
use cherenkov::{BlendMode, Draw as _, FilterId, LayerId, Picture, Prop, ShapeData, SurfaceTree};

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
    tree.note_installed(BELOW, true);
    tree.note_installed(PARENT, true);
    tree.note_installed(ABOVE, true);
    let candidates = video();
    let ready = candidates.keys().copied().collect();
    let instance = crate::interop::wgpu::Instance::new(crate::interop::wgpu::InstanceDescriptor {
        backends: crate::interop::wgpu::Backends::all(),
        ..crate::interop::wgpu::InstanceDescriptor::new_without_display_handle()
    });
    let Some(adapter) =
        pollster::block_on(instance.enumerate_adapters(crate::interop::wgpu::Backends::all()))
            .into_iter()
            .next()
    else {
        return;
    };
    let Ok((device, _queue)) = pollster::block_on(
        adapter.request_device(&crate::interop::wgpu::DeviceDescriptor::default()),
    ) else {
        return;
    };
    let atlas = Atlas::new(&device, u64::MAX);
    let mut content = FxHashMap::default();
    content.insert(
        BELOW,
        ContentData::new(Picture::record(|c| {
            c.fill(
                Rect::new(-2.0, 0.0, 30.0, 20.0),
                cherenkov::WorkingColor::WHITE,
            );
        })),
    );
    content.insert(
        ABOVE,
        ContentData::new(Picture::record(|c| {
            c.fill(
                Rect::new(2.0, 3.0, 8.0, 9.0),
                cherenkov::WorkingColor::WHITE,
            );
        })),
    );
    let fonts = FxHashMap::default();
    let images = FxHashMap::default();
    let bitmaps = FxHashMap::default();
    let content_bindings = FxHashMap::default();
    let glyphs = GlyphContext {
        atlas: &atlas,
        live_stamp: atlas.live_stamp(),
        fonts: &fonts,
        images: &images,
        bitmaps: &bitmaps,
        content: &content_bindings,
    };
    let mut frame = Frame::default();
    let mut lowering = Lowering::new(&mut frame, SIZE);
    lowering.prepare(&mut content, &glyphs).expect("prepared");
    crate::render::lower::refresh_paint_bounds(&mut content, &fonts).expect("paint bounds");
    let mut scratch = PlanScratch::default();
    let mut output = Plan::default();
    for _ in 0..4 {
        super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
        super::refresh_regions(&tree, &candidates, &mut scratch, &mut output, |layer| {
            content.get(&layer).and_then(|content| content.paint_bounds)
        });
    }
    let region_buffers: Vec<_> = output.regions.iter().map(Vec::as_ptr).collect();
    for x in 0..8 {
        tree.apply(LayerOp::Transform(
            VIDEO,
            prop(Affine::translate((f64::from(x), 0.))),
        ));
        start_tracking();
        super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
        super::refresh_regions(&tree, &candidates, &mut scratch, &mut output, |layer| {
            content.get(&layer).and_then(|content| content.paint_bounds)
        });
        let counts = stop_tracking();
        assert_eq!(counts, (0, 0), "pose {x}");
        assert_eq!(
            output.regions.iter().map(Vec::as_ptr).collect::<Vec<_>>(),
            region_buffers,
            "region buffers were retained at pose {x}"
        );
    }
    assert_eq!(
        output.planes,
        plan::<Test>(&tree, &candidates, &ready).planes
    );
}

#[test]
fn hit_regions_use_transformed_paint_and_plane_bounds_intersected_with_clips() {
    let mut tree = scene();
    tree.note_installed(BELOW, true);
    tree.note_installed(PARENT, true);
    tree.note_installed(ABOVE, true);
    tree.apply(LayerOp::Transform(
        BELOW,
        prop(Affine::translate((10.0, 20.0))),
    ));
    tree.apply(LayerOp::Clip(
        BELOW,
        Some(ShapeData::Rect(Rect::new(0.0, 0.0, 20.0, 10.0))),
    ));
    tree.apply(LayerOp::Transform(
        VIDEO,
        prop(Affine::translate((40.0, 50.0))),
    ));
    tree.apply(LayerOp::Clip(
        VIDEO,
        Some(ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 100.0))),
    ));
    let candidates = video();
    let ready = all_ready(&candidates);
    let mut scratch = PlanScratch::default();
    let mut output = Plan::default();
    super::plan_with::<Test>(&tree, &candidates, &ready, &mut scratch, &mut output);
    super::refresh_regions(
        &tree,
        &candidates,
        &mut scratch,
        &mut output,
        |layer| match layer {
            BELOW => Some(Rect::new(-2.0, 0.0, 30.0, 20.0)),
            ABOVE => Some(Rect::new(2.0, 3.0, 8.0, 9.0)),
            _ => None,
        },
    );

    assert_eq!(
        output.regions[0],
        [Some(Rect::new(10.0, 20.0, 30.0, 30.0)), None]
    );
    assert_eq!(output.regions[1], [Some(Rect::new(2.0, 3.0, 8.0, 9.0))]);
    assert_eq!(
        output.plane_regions,
        [Some(Rect::new(40.0, 50.0, 140.0, 150.0))]
    );
}

const fn prop<T>(target: T) -> Prop<T> {
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
    tree.apply(LayerOp::Clip(
        PARENT,
        Some(ShapeData::RoundedRect(RoundedRect::new(
            0.0, 0.0, 72.0, 48.0, 6.0,
        ))),
    ));
    // Scaled 2×, the 48×32 video's content covers (12, 8)–(108, 72); the
    // holder's clip leaves only (12, 8)–(84, 56) on the plane.
    tree.apply(LayerOp::Transform(VIDEO, prop(Affine::scale(2.0))));
    tree.apply(LayerOp::Opacity(ABOVE, prop(0.5)));
    tree.apply(LayerOp::Clip(
        ABOVE,
        Some(ShapeData::Rect(Rect::new(8.0, 58.0, 88.0, 64.0))),
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
    tree.apply(LayerOp::Clip(
        PARENT,
        Some(ShapeData::Circle(kurbo::Circle::new((10.0, 10.0), 5.0))),
    ));
    assert_eq!(verdict(&tree), Err(Ineligible::Clip(PARENT)));
    let mut tree = scene();
    tree.apply(LayerOp::Clip(
        VIDEO,
        Some(ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0))),
    ));
    assert!(verdict(&tree).is_ok());
}

/// Two shaped clips on the path make the engine isolate the inner one into
/// a clip offscreen; a device-aligned rect nests freely.
#[test]
fn nested_shaped_clips_are_not_promoted() {
    let rounded = ShapeData::RoundedRect(RoundedRect::new(0.0, 0.0, 100.0, 50.0, 8.0));
    let mut tree = scene();
    tree.apply(LayerOp::Clip(PARENT, Some(rounded.clone())));
    tree.apply(LayerOp::Clip(VIDEO, Some(rounded.clone())));
    assert_eq!(verdict(&tree), Err(Ineligible::NestedClip(VIDEO)));
    let mut tree = scene();
    tree.apply(LayerOp::Clip(PARENT, Some(rounded)));
    tree.apply(LayerOp::Clip(
        VIDEO,
        Some(ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0))),
    ));
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
    tree.apply(LayerOp::Backdrop(VIDEO, Some(group.sample())));
    assert_eq!(verdict(&tree), Err(Ineligible::Backdrop));
    let mut tree = scene();
    tree.apply(LayerOp::Backdrop(ABOVE, Some(group.sample())));
    assert_eq!(verdict(&tree), Err(Ineligible::BackdropAbove(ABOVE)));
    let mut tree = scene();
    tree.apply(LayerOp::Backdrop(BELOW, Some(group.sample())));
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
    tree.apply(LayerOp::Clip(
        PARENT,
        Some(ShapeData::Circle(kurbo::Circle::new((10.0, 10.0), 5.0))),
    ));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Clip(PARENT)));
    let mut tree = scene();
    tree.apply(LayerOp::Clip(
        PARENT,
        Some(ShapeData::RoundedRect(RoundedRect::new(
            0.0, 0.0, 100.0, 50.0, 8.0,
        ))),
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
    tree.apply(LayerOp::Clip(PARENT, Some(rounded.clone())));
    tree.apply(LayerOp::Clip(VIDEO, Some(rounded)));
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
    tree.apply(LayerOp::Clip(
        VIDEO,
        Some(ShapeData::Rect(Rect::new(0.0, 0.0, 100.0, 50.0))),
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
    tree.apply(LayerOp::Backdrop(VIDEO, Some(group.sample())));
    assert_eq!(hosted_verdict::<Test>(&tree), Err(Ineligible::Backdrop));
    let mut tree = scene();
    tree.apply(LayerOp::Backdrop(ABOVE, Some(group.sample())));
    assert_eq!(
        hosted_verdict::<Test>(&tree),
        Err(Ineligible::BackdropAbove(ABOVE))
    );
    let mut tree = scene();
    tree.apply(LayerOp::Backdrop(BELOW, Some(group.sample())));
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
