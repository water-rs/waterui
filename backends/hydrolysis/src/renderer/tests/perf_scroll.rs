//! Virtual-scroll / lazy-loading performance regression harness.
//!
//! Mirrors the `water preview perf` budget methodology (frame-economy counters:
//! rebuilt frames and measurement-cache misses) as a deterministic in-crate test.
//! The guarantee under test is the defining property of working virtualization:
//! the cost of building and scrolling a lazy list is bounded by the number of
//! *visible* rows, **independent of the total row count**. A regression that
//! materializes or re-measures the whole collection would make per-frame work
//! scale with the total — which these tests fail on.

use core::time::Duration;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_core::layout::Point;
use waterui_core::{AnyView, Dynamic};
use waterui_layout::scroll::{ScrollController, scroll};
use waterui_layout::stack::{VStack, vstack};

#[cfg(feature = "accessibility")]
use waterui::Binding;
use waterui::Str;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::prelude::{FlowAnimationPreset, flow_markdown};

use super::test_environment;
use crate::HeadlessRuntime;
use crate::platform::InputEvent;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 640;
const ROW_HEIGHT: f32 = 44.0;
const SCROLL_FRAMES: u32 = 24;
/// Frames to run an indexed jump for. The approach eases with a ~180ms time
/// constant, so this is comfortably past the point it snaps to its target.
const FRAMES_TO_SETTLE_JUMP: u32 = 60;

/// A vertically-scrolling lazy list of `rows` fixed-height rows.
fn lazy_list(rows: usize) -> AnyView {
    let data = (0..rows).map(SelfId::new).collect::<Vec<_>>();
    AnyView::new(scroll(VStack::for_each(data, |_| {
        ().size(360.0, ROW_HEIGHT)
    })))
}

fn runtime_for(rows: usize) -> HeadlessRuntime {
    let builder = AnyViewBuilder::<AnyView>::new(move || lazy_list(rows));
    let env = test_environment();
    HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT)
}

/// Frame-economy totals over an initial build plus a fixed scroll sequence.
/// Both are re-measurement counts (`measurement_cache_misses`): the per-frame
/// work the visible window actually performed.
#[derive(Debug, Clone, Copy)]
struct ScrollCost {
    initial_misses: u32,
    scroll_misses: u32,
}

fn measure_scroll_cost(rows: usize) -> ScrollCost {
    let mut runtime = runtime_for(rows);
    let start = Instant::now();
    let initial = runtime.pump_at(false, start);
    let initial_misses = initial.profile.counters.measurement_cache_misses;

    let mut scroll_misses = 0;
    for frame in 1..=SCROLL_FRAMES {
        runtime.push_input_event(InputEvent::Scroll {
            x: (WINDOW_WIDTH / 2) as f32,
            y: (WINDOW_HEIGHT / 2) as f32,
            dx: 0.0,
            dy: -200.0,
            is_line_delta: false,
        });
        let result = runtime.pump_at(false, start + Duration::from_millis(u64::from(frame) * 16));
        scroll_misses += result.profile.counters.measurement_cache_misses;
    }

    ScrollCost {
        initial_misses,
        scroll_misses,
    }
}

#[test]
fn lazy_scroll_cost_is_independent_of_total_rows() {
    let small = measure_scroll_cost(500);
    let large = measure_scroll_cost(20_000);
    eprintln!("perf_scroll small(500)={small:?}");
    eprintln!("perf_scroll large(20000)={large:?}");

    // 40x more rows must not blow up per-frame work: virtualization bounds both
    // the initial build and each scroll frame to the visible window.
    assert!(
        large.initial_misses <= small.initial_misses + 64,
        "initial build cost scaled with total rows (virtualization regression): {small:?} vs {large:?}"
    );
    assert!(
        large.scroll_misses <= small.scroll_misses * 2 + 64,
        "scroll cost scaled with total rows (virtualization regression): {small:?} vs {large:?}"
    );
}

#[test]
fn coordinate_jump_over_lazy_stack_materializes_only_the_target_window() {
    const ROWS: usize = 100_000;
    const TARGET_ROW: usize = 90_000;

    let materialized = Arc::new(AtomicUsize::new(0));
    let controller = ScrollController::new(Point::zero());
    let builder = {
        let materialized = Arc::clone(&materialized);
        let controller = controller.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let materialized = Arc::clone(&materialized);
            AnyView::new(
                scroll(VStack::for_each(rows, move |_| {
                    materialized.fetch_add(1, Ordering::Relaxed);
                    ().size(360.0, ROW_HEIGHT)
                }))
                .scroll_controller(&controller),
            )
        })
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    let initial_materialized = materialized.load(Ordering::Relaxed);

    controller.scroll_to(Point::new(0.0, TARGET_ROW as f32 * ROW_HEIGHT));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    let jump_materialized = materialized.load(Ordering::Relaxed) - initial_materialized;

    assert!(
        initial_materialized < 64,
        "initial lazy viewport materialized {initial_materialized} rows"
    );
    assert!(
        jump_materialized < 64,
        "deep coordinate jump materialized {jump_materialized} rows instead of one viewport"
    );
}

#[test]
fn indexed_list_jump_materializes_only_the_target_window() {
    const ROWS: usize = 100_000;
    const TARGET_ROW: usize = 90_000;

    let materialized = Arc::new(AtomicUsize::new(0));
    let controller = ScrollController::new(0usize);
    let builder = {
        let materialized = Arc::clone(&materialized);
        let controller = controller.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
            let materialized = Arc::clone(&materialized);
            AnyView::new(
                List::for_each(rows, move |row| {
                    let index = row.into_inner();
                    materialized.fetch_add(1, Ordering::Relaxed);
                    let label = if index == TARGET_ROW {
                        "Target row"
                    } else {
                        "Dataset row"
                    };
                    ListItem::new(vstack((text(label),)).size(360.0, ROW_HEIGHT))
                })
                .scroll_controller(&controller),
            )
        })
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);
    let initial_materialized = materialized.load(Ordering::Relaxed);

    // An indexed jump eases into place instead of teleporting, so the target
    // arrives over several frames rather than on the next one. Only the final
    // approach is animated — a distant target is closed instantly first — so
    // the whole jump still costs a bounded number of viewports, which is what
    // this test is really guarding.
    controller.scroll_to(TARGET_ROW);
    #[cfg_attr(
        not(feature = "accessibility"),
        allow(unused_mut, unused_variables, reason = "labels need the a11y tree")
    )]
    let mut saw_target = false;
    let mut elapsed = Duration::ZERO;
    let mut previous = initial_materialized;
    let mut worst_frame = 0usize;
    for _ in 0..FRAMES_TO_SETTLE_JUMP {
        elapsed += Duration::from_millis(16);
        let frame = runtime.pump_at(false, start + elapsed);
        let total = materialized.load(Ordering::Relaxed);
        worst_frame = worst_frame.max(total - previous);
        previous = total;
        #[cfg(feature = "accessibility")]
        {
            saw_target |= frame
                .tree_update
                .into_iter()
                .flat_map(|update| update.nodes)
                .any(|(_, node)| node.label() == Some("Target row"));
        }
        #[cfg(not(feature = "accessibility"))]
        let _ = frame;
    }

    assert!(
        initial_materialized < 128,
        "initial List viewport materialized {initial_materialized} rows"
    );
    // The jump eases in, so cost is spread over frames rather than landing on
    // one. What must hold is that no single frame ever materializes more than a
    // viewport: that is what proves the glide is not dragging the list through
    // the 90,000 rows between here and the target.
    assert!(
        worst_frame < 128,
        "a frame of the indexed List jump materialized {worst_frame} rows instead of one viewport"
    );
    #[cfg(feature = "accessibility")]
    assert!(
        saw_target,
        "the indexed target row must intersect the viewport once the jump settles"
    );
}

#[test]
fn visible_lazy_item_remeasures_connected_dynamic_through_retained_node() {
    let (handler, dynamic) = Dynamic::new();
    handler.set(text("initial"));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        let dynamic = dynamic.clone();
        AnyView::new(scroll(VStack::for_each(
            vec![SelfId::new(0_u64)],
            move |_| dynamic.clone(),
        )))
    });
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    handler.set(text("updated dynamic row"));
    let updated = runtime.pump_at(false, start + Duration::from_millis(16));

    assert_eq!(
        updated.profile.counters.rebuild_iterations, 0,
        "a connected Dynamic inside a visible lazy item must patch and remeasure through \
         its retained node without rebuilding the window"
    );
    #[cfg(feature = "accessibility")]
    assert!(
        updated
            .tree_update
            .into_iter()
            .flat_map(|update| update.nodes)
            .any(|(_, node)| node.label() == Some("updated dynamic row")),
        "the patched lazy item must lay out and publish its replacement subtree"
    );
}

#[test]
fn flow_markdown_blocks_reconnect_after_lazy_eviction() {
    let markdown = (0..64)
        .map(|index| {
            format!(
                "# Section {index}\n\nParagraph {index} keeps the document tall enough to recycle blocks.\n\n"
            )
        })
        .collect::<String>();
    let controller = ScrollController::new(Point::zero());
    let builder = {
        let controller = controller.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(
                scroll(
                    flow_markdown(Str::from(markdown.clone())).preset(FlowAnimationPreset::None),
                )
                .scroll_controller(&controller),
            )
        })
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    controller.scroll_to(Point::new(0.0, 3_000.0));
    let _ = runtime.pump_at(false, start + Duration::from_millis(16));
    controller.scroll_to(Point::zero());
    let returned = runtime.pump_at(false, start + Duration::from_millis(32));
    #[cfg(not(feature = "accessibility"))]
    let _ = returned;

    #[cfg(feature = "accessibility")]
    assert!(
        returned
            .tree_update
            .into_iter()
            .flat_map(|update| update.nodes)
            .any(|(_, node)| node.label() == Some("Section 0")),
        "a FlowMarkdown block rematerialized after lazy eviction must reconnect with fresh retained content"
    );
}

#[cfg(feature = "accessibility")]
#[test]
fn flow_markdown_append_preserves_user_scroll_offset() {
    fn scroll_y(result: crate::HeadlessPumpResult) -> f64 {
        result
            .tree_update
            .expect("FlowMarkdown pump must publish an accessibility update")
            .nodes
            .into_iter()
            .find_map(|(_, node)| node.scroll_y())
            .expect("FlowMarkdown scroll view must publish its vertical offset")
    }

    let initial = (0..64)
        .map(|index| {
            format!(
                "# Section {index}\n\nParagraph {index} keeps the document tall enough to retain a user offset.\n\n"
            )
        })
        .collect::<String>();
    let source = Binding::container(Str::from(initial.clone()));
    let builder = {
        let source = source.clone();
        AnyViewBuilder::<AnyView>::new(move || {
            AnyView::new(scroll(
                flow_markdown(source.clone()).preset(FlowAnimationPreset::None),
            ))
        })
    };
    let env = test_environment();
    let mut runtime = HeadlessRuntime::new_for_tests(env, builder, WINDOW_WIDTH, WINDOW_HEIGHT);
    let start = Instant::now();
    let _ = runtime.pump_at(false, start);

    runtime.push_input_event(InputEvent::Scroll {
        x: WINDOW_WIDTH as f32 / 2.0,
        y: WINDOW_HEIGHT as f32 / 2.0,
        dx: 0.0,
        dy: -120.0,
        is_line_delta: false,
    });
    let before_append = scroll_y(runtime.pump_at(false, start + Duration::from_millis(16)));
    assert!(
        before_append > 0.0,
        "the physical scroll input must establish a non-zero user offset"
    );

    source.set(Str::from(format!("{initial}\n\nAppended tail block.")));
    let after_append = scroll_y(runtime.pump_at(false, start + Duration::from_millis(32)));
    assert_eq!(
        before_append, after_append,
        "incremental FlowMarkdown growth must not reset the user's scroll offset"
    );
}
