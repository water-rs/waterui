//! Frame-economy regression harness for the Hydrolysis retained-scene refactor.
//!
//! These tests drive the full headless frame pump over representative animated
//! content and assert on [`FrameCounters`]. They lock in the frame economy so
//! that parametric updates (animation ticks, scroll offset changes) stop forcing
//! whole-window re-dispatch and re-measurement.
//!
//! The legacy engine only avoids a full rebuild for one narrow case: an animated
//! *transform* (scale/rotation/offset) inside a single window-filling scroll view,
//! which it captures as a replayable `DynamicTransformDraw`. Every other animation
//! — a non-transform property such as opacity, or any animation at the window root
//! with no retained scroll frame — falls back to a full scene rebuild (and full
//! re-measure) on every frame. That is the regression this refactor eliminates.

use core::time::Duration;
use std::time::Instant;

use waterui::animation::Animation;
use waterui::{Binding, SignalExt as _, ViewExt as _};
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::scroll;
use waterui_layout::stack::{VStack, vstack, zstack};

use nami::collection::List;
use waterui::graphics::Color;
use waterui_core::dynamic::watch;
use waterui_core::views::ForEach;
use waterui_layout::AbsoluteLayout;
use waterui_layout::collection_transition::collection_transition;
use waterui_layout::container::LazyContainer;

use super::test_environment;
use crate::HeadlessRuntime;

/// Aggregated frame-economy metrics over a run of parametric (post-trigger) frames.
#[derive(Debug, Clone, Copy, Default)]
struct ScenarioMetrics {
    /// Number of frames pumped while the animation was active.
    parametric_frames: u32,
    /// How many of those frames performed at least one full scene rebuild.
    frames_rebuilt: u32,
    /// Total measurement-cache misses across the parametric frames (re-measure cost).
    measurement_misses: u32,
}

/// A long scrollable list of fixed-size rows, used to pad scenarios so the scroll
/// view is the exclusive window-filling root the legacy fast-path expects.
fn padding_list() -> impl waterui::View {
    let rows = (0..40).map(SelfId::new).collect::<Vec<_>>();
    VStack::for_each(rows, |_| ().size(360.0, 44.0))
}

/// Case A — animated **opacity at the window root** (no scroll frame at all).
/// Legacy: rebuilds the whole scene every animation frame.
fn opacity_at_root(value: &Binding<f32>) -> AnyView {
    let animated = value
        .clone()
        .with(Animation::linear(Duration::from_millis(1_000)));
    AnyView::new(vstack((
        ().size(120.0, 120.0).opacity(animated),
        ().size(360.0, 200.0),
    )))
}

/// Case B — animated **opacity inside a scroll view**. The opacity is baked into
/// the captured scroll content, so the legacy cache is `animation_dependent` and
/// bails to a full rebuild every frame.
fn opacity_in_scroll(value: &Binding<f32>) -> AnyView {
    let animated = value
        .clone()
        .with(Animation::linear(Duration::from_millis(1_000)));
    let header = ().size(120.0, 120.0).opacity(animated);
    AnyView::new(scroll(vstack((header, padding_list()))))
}

/// Case A' — animated **transform (scale) at the window root** (no scroll frame).
/// Legacy bakes root transforms and rebuilds every frame; the window retained frame
/// must replay it without a rebuild.
fn transform_at_root(value: &Binding<f32>) -> AnyView {
    let animated = value
        .clone()
        .with(Animation::linear(Duration::from_millis(1_000)));
    AnyView::new(vstack((
        ().size(80.0, 80.0).scale(animated.clone(), animated),
        ().size(360.0, 200.0),
    )))
}

/// Case C — animated **transform (scale) inside a scroll view**. This is the one
/// case the legacy retained-scroll path already replays without a rebuild; it
/// exists here as a no-regression guard for the refactor.
fn transform_in_scroll(value: &Binding<f32>) -> AnyView {
    let animated = value
        .clone()
        .with(Animation::linear(Duration::from_millis(1_000)));
    let header = ().size(80.0, 80.0).scale(animated.clone(), animated);
    AnyView::new(scroll(vstack((header, padding_list()))))
}

/// Pumps one initial structural frame, triggers an animation by moving `value`,
/// then pumps `parametric_frames` further frames while the animation is active and
/// records the frame economy.
fn run_scenario(
    make_view: fn(&Binding<f32>) -> AnyView,
    parametric_frames: u32,
) -> ScenarioMetrics {
    let value = Binding::f32(1.0);
    let builder = {
        let value = value.clone();
        AnyViewBuilder::<AnyView>::new(move || make_view(&value))
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);

    let start = Instant::now();
    // Initial structural build.
    let _ = runtime.pump_at(false, start);

    // Start an animation: the bound value animates from 1.0 -> 0.25 over 1s.
    value.set(0.25);

    let mut metrics = ScenarioMetrics::default();
    for frame in 1..=parametric_frames {
        let at = start + Duration::from_millis(u64::from(frame) * 16);
        let result = runtime.pump_at(false, at);
        let counters = result.profile.counters;
        metrics.parametric_frames += 1;
        if counters.rebuild_iterations > 0 {
            metrics.frames_rebuilt += 1;
        }
        metrics.measurement_misses += counters.measurement_cache_misses;
    }
    metrics
}

/// A `Dynamic` node whose content changes color (but keeps a fixed size) with `value`,
/// sitting above a long static list. A content change here keeps the surrounding layout
/// stable, so it must be applied as an isolated reactive patch.
fn dynamic_same_size(value: &Binding<i32>) -> AnyView {
    let watched = watch(value.clone(), |v| {
        let channel = if v % 2 == 0 { 40 } else { 200 };
        Color::srgb(channel, 90, 160).size(80.0, 40.0)
    });
    AnyView::new(vstack((watched, padding_list())))
}

fn dynamic_runtime(
    make_view: fn(&Binding<i32>) -> AnyView,
    value: &Binding<i32>,
) -> HeadlessRuntime {
    let builder = {
        let value = value.clone();
        AnyViewBuilder::<AnyView>::new(move || make_view(&value))
    };
    let env = test_environment();
    HeadlessRuntime::new_for_tests(env, builder, 400, 640)
}

/// Phase 3: a same-size content change to one `Dynamic` node is applied as an isolated
/// reactive patch — only that node is re-dispatched, and the frame is re-composited
/// without any structural rebuild of the rest of the window.
#[test]
fn dynamic_content_change_patches_without_rebuild() {
    let value = Binding::container(0_i32);
    let mut runtime = dynamic_runtime(dynamic_same_size, &value);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    value.set(1);
    let result = runtime.pump_at(false, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "a same-size Dynamic content change must patch in isolation, not rebuild: {:?}",
        result.profile.counters
    );
    assert!(
        !result.rebuilt,
        "a reactive patch frame must not perform a structural rebuild"
    );
}

/// A `Dynamic` whose content changes *height* and paints a visible colour, above a
/// long list of visible rows, so a reflow is observable in pixels.
fn dynamic_changing_size_visible(value: &Binding<i32>) -> AnyView {
    let watched = watch(value.clone(), |v| {
        Color::srgb(200, 90, 160).size(80.0, 40.0 + (v as f32) * 30.0)
    });
    let rows = (0..40).map(SelfId::new).collect::<Vec<_>>();
    let list = VStack::for_each(rows, |item: SelfId<u64>| {
        let channel = u8::try_from(40 + (*item % 8) * 24).unwrap_or(255);
        Color::srgb(channel, 120, 80).size(360.0, 44.0)
    });
    AnyView::new(vstack((watched, list)))
}

/// The retained tree reflows a size-changing `Dynamic` content change incrementally
/// — it relays out the persistent tree in place (the surrounding rows shift down)
/// without a whole-window structural rebuild. This is the fix for the chart's layout
/// flicker (Bug 2): the legacy engine reset the scene and re-dispatched the whole
/// window for any size-affecting change, which flashed. The reflowed frame must be
/// pixel-identical to composing the same content statically.
#[test]
fn dynamic_size_change_reflows_without_rebuild() {
    // Ground truth: the same composition built statically at the final value.
    let truth_value = Binding::container(1_i32);
    let truth_builder = {
        let value = truth_value.clone();
        AnyViewBuilder::<AnyView>::new(move || dynamic_changing_size_visible(&value))
    };
    let truth_env = test_environment();
    let mut truth_runtime = HeadlessRuntime::new_for_tests(truth_env, truth_builder, 400, 640);
    let expected = truth_runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("ground-truth frame must produce a snapshot");

    let value = Binding::container(0_i32);
    let builder = {
        let value = value.clone();
        AnyViewBuilder::<AnyView>::new(move || dynamic_changing_size_visible(&value))
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);
    let start = Instant::now();
    let _ = runtime.pump_at(true, start);

    value.set(1);
    let result = runtime.pump_at(true, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "a size-changing Dynamic change must reflow via incremental relayout, not a \
         whole-window structural rebuild (Bug 2 — the flicker): {:?}",
        result.profile.counters
    );
    let snapshot = result
        .snapshot
        .expect("reflowed frame must produce a snapshot");
    assert!(
        snapshot.rgba8 == expected.rgba8,
        "the incremental reflow must place the grown content and shifted rows exactly \
         as a static composition would; a mismatch means the in-place relayout is wrong"
    );
}

/// A reactive collection (`ForEach`/`List`) in a full-window `AbsoluteLayout`
/// overlay, mirroring the snackbar overlay: each item is a fixed-size box keyed
/// by id. Membership changes must reconcile in isolation (the collection's size
/// is constant — `AbsoluteLayout` fills the window regardless of item count — so
/// no surrounding reflow is needed).
fn collection_overlay(list: &List<SelfId<u64>>) -> AnyView {
    let list = list.clone();
    AnyView::new(zstack((
        ().size(360.0, 600.0),
        LazyContainer::new(
            AbsoluteLayout,
            ForEach::new(list, |item: SelfId<u64>| {
                // Colour by id so the topmost (last) item — and thus the composited
                // pixels — changes when membership changes. `AbsoluteLayout` stacks
                // every item at the same origin, so the last opaque item shows.
                let id = u8::try_from(*item % 4).unwrap_or(0);
                Color::srgb(40 + id * 50, 90, 160).size(80.0, 40.0)
            }),
        ),
    )))
}

fn collection_runtime(list: &List<SelfId<u64>>) -> HeadlessRuntime {
    let builder = {
        let list = list.clone();
        AnyViewBuilder::<AnyView>::new(move || collection_overlay(&list))
    };
    let env = test_environment();
    HeadlessRuntime::new_for_tests(env, builder, 400, 640)
}

/// Adding an item to a reactive collection in a constant-size overlay reconciles
/// as an isolated patch — only the new item is dispatched, and the window is
/// re-composited without any structural rebuild of the rest of the tree.
#[test]
fn collection_add_patches_without_rebuild() {
    let list: List<SelfId<u64>> = List::new();
    let mut runtime = collection_runtime(&list);
    let start = Instant::now();
    let empty = runtime
        .pump_at(true, start)
        .snapshot
        .expect("empty overlay frame must produce a snapshot");

    list.push(SelfId::new(1));
    let result = runtime.pump_at(true, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "adding a collection item must patch in isolation, not rebuild: {:?}",
        result.profile.counters
    );
    assert!(
        !result.rebuilt,
        "a collection add patch frame must not perform a structural rebuild"
    );
    let added = result
        .snapshot
        .expect("collection-add frame must produce a snapshot");
    assert!(
        added.rgba8 != empty.rgba8,
        "the added item must actually appear (the reactive collection must reconcile \
         the new id into the rendered tree, not just avoid a rebuild)"
    );
}

/// Removing an item from a reactive collection in a constant-size overlay also
/// reconciles as an isolated patch (the removed item's retained subtree is
/// evicted; survivors keep theirs).
#[test]
fn collection_remove_patches_without_rebuild() {
    let list: List<SelfId<u64>> = List::from(vec![SelfId::new(1), SelfId::new(2), SelfId::new(3)]);
    let mut runtime = collection_runtime(&list);
    let start = Instant::now();
    let three = runtime
        .pump_at(true, start)
        .snapshot
        .expect("three-item overlay frame must produce a snapshot");

    // Remove the last (topmost) item so the composited overlay pixels change.
    let _removed = list.remove(2);
    let result = runtime.pump_at(true, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "removing a collection item must patch in isolation, not rebuild: {:?}",
        result.profile.counters
    );
    assert!(
        !result.rebuilt,
        "a collection remove patch frame must not perform a structural rebuild"
    );
    let two = result
        .snapshot
        .expect("collection-remove frame must produce a snapshot");
    assert!(
        two.rgba8 != three.rgba8,
        "the removed item must actually disappear (the reactive collection must evict \
         the departed id from the rendered tree)"
    );
}

/// Phase 4: scrolling a fixed (non-lazy) list re-composites the retained window frame at
/// the new offset — the scroll fast-path is subsumed into the window frame — with no
/// structural rebuild.
#[test]
fn fixed_scroll_refreshes_window_frame_without_rebuild() {
    use crate::platform::InputEvent;

    let builder = AnyViewBuilder::<AnyView>::new(|| {
        AnyView::new(scroll(vstack((
            ().size(360.0, 1_200.0),
            ().size(360.0, 200.0),
        ))))
    });
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);

    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(InputEvent::Scroll {
        x: 200.0,
        y: 320.0,
        dx: 0.0,
        dy: -120.0,
        is_line_delta: false,
    });
    let result = runtime.pump_at(false, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "fixed scroll must re-composite via the window frame, not rebuild: {:?}",
        result.profile.counters
    );
    assert!(
        !result.rebuilt,
        "a fixed scroll refresh frame must not perform a structural rebuild"
    );
}

const PARAMETRIC_FRAMES: u32 = 20;

/// Phase 2B: an opacity animation at the window root is captured as a replayable
/// dynamic opacity layer and replays through the window frame without a rebuild or
/// re-measure (legacy baked opacity and rebuilt every frame).
#[test]
fn opacity_at_root_never_rebuilds() {
    let metrics = run_scenario(opacity_at_root, PARAMETRIC_FRAMES);
    assert_eq!(metrics.parametric_frames, PARAMETRIC_FRAMES);
    assert_eq!(
        metrics.frames_rebuilt, 0,
        "root opacity animation must replay through the window frame, not rebuild: {metrics:?}"
    );
    assert_eq!(
        metrics.measurement_misses, 0,
        "root opacity animation must not re-measure the tree: {metrics:?}"
    );
}

/// Phase 2B: an opacity animation inside a scroll view replays through the retained
/// scroll cache without a rebuild — the case that previously forced a full rebuild
/// (and the flashing) on every animation/scroll frame.
#[test]
fn opacity_in_scroll_never_rebuilds() {
    let metrics = run_scenario(opacity_in_scroll, PARAMETRIC_FRAMES);
    assert_eq!(metrics.parametric_frames, PARAMETRIC_FRAMES);
    assert_eq!(
        metrics.frames_rebuilt, 0,
        "in-scroll opacity animation must replay, not rebuild every frame: {metrics:?}"
    );
    assert_eq!(
        metrics.measurement_misses, 0,
        "in-scroll opacity animation must not re-measure the tree: {metrics:?}"
    );
}

/// Phase 2A: an animated transform at the window root replays through the retained
/// window frame without any structural rebuild or re-measure.
#[test]
fn transform_at_root_never_rebuilds() {
    let metrics = run_scenario(transform_at_root, PARAMETRIC_FRAMES);
    assert_eq!(metrics.parametric_frames, PARAMETRIC_FRAMES);
    assert_eq!(
        metrics.frames_rebuilt, 0,
        "root transform animation must replay through the window frame, not rebuild: {metrics:?}"
    );
    assert_eq!(
        metrics.measurement_misses, 0,
        "root transform animation must not re-measure the tree: {metrics:?}"
    );
}

/// NO-REGRESSION GUARD: an animated transform inside a window-filling scroll already
/// replays without a rebuild today. The refactor must keep it at zero rebuilds.
#[test]
fn transform_in_scroll_never_rebuilds() {
    let metrics = run_scenario(transform_in_scroll, PARAMETRIC_FRAMES);
    assert_eq!(metrics.parametric_frames, PARAMETRIC_FRAMES);
    assert_eq!(
        metrics.frames_rebuilt, 0,
        "animated transform inside scroll must replay without a full rebuild: {metrics:?}"
    );
    assert_eq!(
        metrics.measurement_misses, 0,
        "animated transform inside scroll must not re-measure: {metrics:?}"
    );
}

/// A size-changing `Dynamic` patch reflows the new content at its grown bounds — not
/// replay a capture made under the stale placement — without a whole-window rebuild.
/// An overlay-style `Dynamic` that grows from empty (zero-size) to visible content is the
/// canonical case (snackbar presentation): the retained tree rebuilds only that node's
/// child and relays out in place, and the grown frame must render pixel-identical to
/// composing the same content statically.
#[test]
fn dynamic_growth_from_empty_renders_content_without_rebuild() {
    use waterui::component::text;
    use waterui_core::Dynamic;

    fn overlay_content() -> AnyView {
        AnyView::new(text("presented overlay"))
    }

    // Ground truth: the same composition built statically.
    let static_builder = AnyViewBuilder::<AnyView>::new(|| {
        AnyView::new(zstack((().size(360.0, 600.0), overlay_content())))
    });
    let static_env = test_environment();
    let mut static_runtime = HeadlessRuntime::new_for_tests(static_env, static_builder, 400, 640);
    let expected = static_runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("static overlay frame must produce a snapshot");

    // Mirror the runtime window composition: overlay Dynamic layered in a ZStack
    // above the main content, sized by its own (initially zero) measurement.
    let (handler, dynamic) = Dynamic::new();
    handler.set(());
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        AnyView::new(zstack((().size(360.0, 600.0), dynamic.clone())))
    });
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);

    let start = Instant::now();
    let _ = runtime.pump_at(true, start);

    handler.set(overlay_content());
    let result = runtime.pump_at(true, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "growing a Dynamic from empty must reflow via incremental relayout of the \
         retained tree, not a whole-window structural rebuild: {:?}",
        result.profile.counters
    );
    let snapshot = result
        .snapshot
        .expect("reflowed overlay frame must produce a snapshot");
    assert!(
        snapshot.rgba8 == expected.rgba8,
        "the reflow must place the overlay content at its grown bounds; a mismatch with \
         the statically composed frame means a stale zero-size capture was replayed"
    );
}

/// Each item paints a reactive background driven by `selected` (the selected id
/// fills `ACTIVE`, the rest fill `INACTIVE`), inside a `collection_transition`,
/// which routes the `VStack` collection to the retained per-item path — the same
/// path the navigation-drawer active-indicator pill uses. A sibling whose *height*
/// tracks the same `selected` binding forces a selection change to reflow the
/// layout and escalate to a structural rebuild, which reuses the collection's item
/// subtrees rather than re-dispatching them.
fn reactive_bg_collection(selected: &Binding<u32>) -> AnyView {
    let list: List<SelfId<u64>> = List::from(vec![SelfId::new(0), SelfId::new(1), SelfId::new(2)]);
    let sel = selected.clone();
    let collection = VStack::for_each(list, move |item: SelfId<u64>| {
        let id = *item as u32;
        let is_selected = sel.clone().map(move |current| current == id).computed();
        let background = is_selected
            .select(Color::srgb(200, 90, 160), Color::srgb(228, 224, 236))
            .computed();
        AnyView::new(().size(120.0, 40.0).background(background))
    });
    let collection = collection_transition(collection, Animation::linear(Duration::from_millis(1)));
    let sizer = watch(selected.clone(), |s| ().size(80.0, 40.0 + s as f32 * 30.0));
    AnyView::new(vstack((sizer, collection)))
}

/// Regression: a retained collection item's nested reactive content tracks its
/// signal across a sibling-driven reflow. The active-indicator `.background(
/// Computed<Color>)` must repaint when `selected` changes — the navigation drawer's
/// active pill moving on selection — and the size-tracking sibling's reflow must be
/// an incremental relayout of the retained tree, not a whole-window rebuild.
#[test]
fn reused_collection_item_reactive_background_tracks_on_selection() {
    // Ground truth: built fresh with the second item already selected.
    let truth_selected = Binding::container(1_u32);
    let truth_builder = {
        let selected = truth_selected.clone();
        AnyViewBuilder::<AnyView>::new(move || reactive_bg_collection(&selected))
    };
    let truth_env = test_environment();
    let mut truth_runtime = HeadlessRuntime::new_for_tests(truth_env, truth_builder, 400, 640);
    let expected = truth_runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("ground-truth frame must produce a snapshot");

    // Subject: start with the first item selected, then select the second. The
    // size-tracking sibling reflows, escalating the change to a structural rebuild
    // that reuses the collection items.
    let selected = Binding::container(0_u32);
    let builder = {
        let selected = selected.clone();
        AnyViewBuilder::<AnyView>::new(move || reactive_bg_collection(&selected))
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);
    let start = Instant::now();
    let _ = runtime.pump_at(true, start);

    selected.set(1);
    let result = runtime.pump_at(true, start + Duration::from_millis(16));
    assert_eq!(
        result.profile.counters.rebuild_iterations, 0,
        "the size-tracking sibling's reflow must be an incremental relayout of the \
         retained tree, not a whole-window structural rebuild: {:?}",
        result.profile.counters
    );
    let snapshot = result
        .snapshot
        .expect("reflowed frame must produce a snapshot");
    assert!(
        snapshot.rgba8 == expected.rgba8,
        "a retained collection item must apply its nested reactive background update \
         on selection; a mismatch means the active-indicator pill stayed on the stale \
         (previously selected) item"
    );
}

/// A `collection_transition` stack whose rows are distinct solid colors, so a
/// snapshot uniquely identifies which rows are visible and at what extent.
fn transition_color_stack(list: &List<SelfId<u64>>) -> AnyView {
    let collection = VStack::for_each(list.clone(), |item: SelfId<u64>| {
        #[allow(clippy::cast_possible_truncation)]
        let shade = 40 + (*item as u8) * 70;
        AnyView::new(().size(120.0, 40.0).background(Color::srgb(shade, 90, 160)))
    });
    AnyView::new(collection_transition(
        collection,
        Animation::linear(Duration::from_millis(1_000)),
    ))
}

/// Removing an item from a `collection_transition` collection animates it out:
/// a mid-flight frame differs from both the before frame (the row is collapsing)
/// and the settled frame (it has not finished), and after the animation the
/// frame is pixel-identical to a fresh build of the final membership — the
/// exiting row's node is dropped and its space fully released.
#[test]
fn collection_membership_exit_animates_then_settles() {
    // Ground truth: built fresh with the settled membership (rows 0 and 2).
    let truth_list: List<SelfId<u64>> = List::from(vec![SelfId::new(0), SelfId::new(2)]);
    let truth_builder = AnyViewBuilder::<AnyView>::new(move || transition_color_stack(&truth_list));
    let truth_env = test_environment();
    let mut truth_runtime = HeadlessRuntime::new_for_tests(truth_env, truth_builder, 400, 640);
    let expected = truth_runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("ground-truth frame must produce a snapshot");

    // Subject: rows 0, 1, 2; remove the middle row and watch it collapse out.
    let list: List<SelfId<u64>> = List::from(vec![SelfId::new(0), SelfId::new(1), SelfId::new(2)]);
    let builder = {
        let list = list.clone();
        AnyViewBuilder::<AnyView>::new(move || transition_color_stack(&list))
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);
    let start = Instant::now();
    let before = runtime
        .pump_at(true, start)
        .snapshot
        .expect("initial frame must produce a snapshot");

    let _ = list.remove(1);
    // The exit starts on the first frame that processes the membership change.
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    let mid = runtime
        .pump_at(true, start + Duration::from_millis(516))
        .snapshot
        .expect("mid-transition frame must produce a snapshot");
    assert!(
        mid.rgba8 != before.rgba8,
        "~500ms into a 1000ms exit the removed row must be visibly collapsing, \
         not still rendered at rest"
    );
    assert!(
        mid.rgba8 != expected.rgba8,
        "~500ms into a 1000ms exit the removed row must still be partially \
         visible, not already settled"
    );

    let settled = runtime
        .pump_at(true, start + Duration::from_millis(1_616))
        .snapshot
        .expect("settled frame must produce a snapshot");
    assert!(
        settled.rgba8 == expected.rgba8,
        "after the exit animation completes the collection must render \
         pixel-identical to a fresh build of the settled membership"
    );
}

/// Re-adding an item mid-exit turns the exit into a fresh enter (no pop), and
/// an added item fades/grows in: the settled frame matches a fresh build of the
/// grown membership.
#[test]
fn collection_membership_enter_animates_then_settles() {
    // Ground truth: built fresh with all three rows.
    let truth_list: List<SelfId<u64>> =
        List::from(vec![SelfId::new(0), SelfId::new(1), SelfId::new(2)]);
    let truth_builder = AnyViewBuilder::<AnyView>::new(move || transition_color_stack(&truth_list));
    let truth_env = test_environment();
    let mut truth_runtime = HeadlessRuntime::new_for_tests(truth_env, truth_builder, 400, 640);
    let expected = truth_runtime
        .pump_at(true, Instant::now())
        .snapshot
        .expect("ground-truth frame must produce a snapshot");

    // Subject: rows 0 and 2; insert row 1 between them and watch it grow in.
    let list: List<SelfId<u64>> = List::from(vec![SelfId::new(0), SelfId::new(2)]);
    let builder = {
        let list = list.clone();
        AnyViewBuilder::<AnyView>::new(move || transition_color_stack(&list))
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, 400, 640);
    let start = Instant::now();
    let before = runtime
        .pump_at(true, start)
        .snapshot
        .expect("initial frame must produce a snapshot");

    list.insert(1, SelfId::new(1));
    // The enter starts on the first frame that processes the membership change.
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    let mid = runtime
        .pump_at(true, start + Duration::from_millis(516))
        .snapshot
        .expect("mid-transition frame must produce a snapshot");
    assert!(
        mid.rgba8 != before.rgba8,
        "~500ms into a 1000ms enter the inserted row must be visibly growing in"
    );
    assert!(
        mid.rgba8 != expected.rgba8,
        "~500ms into a 1000ms enter the inserted row must not yet be at rest"
    );

    let settled = runtime
        .pump_at(true, start + Duration::from_millis(1_616))
        .snapshot
        .expect("settled frame must produce a snapshot");
    assert!(
        settled.rgba8 == expected.rgba8,
        "after the enter animation completes the collection must render \
         pixel-identical to a fresh build of the grown membership"
    );
}
