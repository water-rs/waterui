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
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_core::{AnyView, Environment};
use waterui_layout::scroll;
use waterui_layout::stack::{VStack, vstack};

use super::MinimalTestTheme;
use crate::HeadlessRuntime;
use crate::engine::WidgetTheme;

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
    let mut env = Environment::new();
    env.insert(Box::new(MinimalTestTheme) as Box<dyn WidgetTheme>);
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
