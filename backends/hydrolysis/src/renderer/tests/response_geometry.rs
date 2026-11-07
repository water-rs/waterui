//! Response geometry on the renderer's own records (water-rs/waterui#1803):
//! a text draws its run in its response rectangle, and a list row positions
//! its content's response at the leading edge of its content rect, centred
//! vertically, while the content keeps the whole rect as its allocation
//! (water-rs/waterui#3888).

use core::time::Duration;
use std::rc::Rc;
use std::time::Instant;

use accesskit::{Rect, Role, TreeUpdate};
use kurbo::Point;
use nami::collection::SignalCollection;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityGeometry;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_core::layout::LayoutDirection;
use waterui_core::{AnyView, Environment};
use waterui_layout::stack::hstack;

use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;

const WINDOW_WIDTH: u32 = 400;
const WINDOW_HEIGHT: u32 = 600;
/// `MinimalTestTheme`'s horizontal row inset.
const ROW_HORIZONTAL_INSET: f64 = 16.0;
const TOLERANCE: f64 = 0.5;

fn runtime(env: Environment, view: impl Fn() -> AnyView + 'static) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        env,
        AnyViewBuilder::<AnyView>::new(view),
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
        MinimalTestTheme::default(),
    )
}

fn rtl_environment() -> Environment {
    let mut env = test_environment();
    env.insert(LayoutDirection::RightToLeft);
    env
}

/// Pumps until the runtime is quiet, then returns the settled tree.
fn settle(runtime: &mut HeadlessRuntime) -> Vec<TreeUpdate> {
    let mut at = Instant::now();
    for frame in 1.. {
        at += Duration::from_millis(16);
        let _ = runtime.pump_at(false, at);
        if frame >= 4
            && (frame >= 300 || (runtime.is_settled() && !runtime.has_pending_semantic_update()))
        {
            break;
        }
    }
    runtime.accessibility_tree().into_iter().collect()
}

fn bounds(updates: &[TreeUpdate], role: Role, label: &str) -> Rect {
    updates
        .iter()
        .rev()
        .find_map(|update| {
            update.nodes.iter().find_map(|(_, node)| {
                (node.role() == role && node.label() == Some(label))
                    .then(|| node.bounds())
                    .flatten()
            })
        })
        .unwrap_or_else(|| {
            let all: Vec<_> = updates
                .iter()
                .flat_map(|update| update.nodes.iter())
                .map(|(_, node)| (node.role(), node.label().map(str::to_owned), node.bounds()))
                .collect();
            panic!("no {role:?} node labelled {label:?} with bounds: {all:?}")
        })
}

/// The window-space origin of every glyph run the painted scene records —
/// where the renderer placed each text layout.
fn run_origins(runtime: &HeadlessRuntime) -> Vec<Point> {
    runtime
        .renderer()
        .painted_scene()
        .glyph_runs()
        .map(|(transform, _)| transform * Point::ORIGIN)
        .collect()
}

const fn centre(rect: Rect) -> (f64, f64) {
    (
        f64::midpoint(rect.x0, rect.x1),
        f64::midpoint(rect.y0, rect.y1),
    )
}

#[track_caller]
fn assert_close(actual: f64, expected: f64, what: &str) {
    assert!(
        (actual - expected).abs() < TOLERANCE,
        "{what}: expected {expected:.2}, got {actual:.2}"
    );
}

/// A text smaller than the window draws its run at its response
/// rectangle's origin — centred in the window — not at the allocation's
/// top-leading corner, and reports that rectangle as its bounds.
#[test]
fn oversized_text_draws_its_run_in_its_response_rectangle() {
    let mut runtime = runtime(test_environment(), || AnyView::new(text("Hello")));
    let updates = settle(&mut runtime);
    let response = bounds(&updates, Role::Label, "Hello");
    assert!(
        response.width() < f64::from(WINDOW_WIDTH) / 2.0
            && response.height() < f64::from(WINDOW_HEIGHT) / 2.0,
        "the response is the text's own size, got {response:?}"
    );
    let (x, y) = centre(response);
    assert_close(x, f64::from(WINDOW_WIDTH) / 2.0, "response centre x");
    assert_close(y, f64::from(WINDOW_HEIGHT) / 2.0, "response centre y");
    assert_runs_at(&runtime, &[response]);
}

fn one_row_list(content: impl Fn() -> AnyView + 'static) -> impl Fn() -> AnyView + 'static {
    let content = Rc::new(content);
    move || {
        let content = content.clone();
        AnyView::new(List::for_each(
            SignalCollection::new(vec![SelfId::new(0_u64)]),
            move |_| ListItem::new(content()),
        ))
    }
}

/// The row's content column: the row inset by the theme's horizontal
/// insets. Its vertical centre is the content rect's.
fn content_column(row: Rect) -> Rect {
    Rect::new(
        row.x0 + ROW_HORIZONTAL_INSET,
        row.y0,
        row.x1 - ROW_HORIZONTAL_INSET,
        row.y1,
    )
}

/// Asserts `response` sits on the leading side of `content` — the left in
/// left-to-right, the right in right-to-left — centred vertically, and is
/// narrower than it.
#[track_caller]
fn assert_leading(response: Rect, content: Rect, direction: LayoutDirection) {
    assert!(
        response.width() < content.width() / 2.0,
        "the content answers less than the row, got {response:?} in {content:?}"
    );
    if direction.is_right_to_left() {
        assert_close(response.x1, content.x1, "response trailing edge");
    } else {
        assert_close(response.x0, content.x0, "response leading edge");
    }
    assert_close(centre(response).1, centre(content).1, "response centre y");
}

/// Asserts each painted run starts at the origin of one of `labels`.
#[track_caller]
fn assert_runs_at(runtime: &HeadlessRuntime, labels: &[Rect]) {
    let origins = run_origins(runtime);
    assert_eq!(
        origins.len(),
        labels.len(),
        "one run per label, got {origins:?}"
    );
    for label in labels {
        assert!(
            origins
                .iter()
                .any(|origin| (origin.x - label.x0).abs() < TOLERANCE
                    && (origin.y - label.y0).abs() < TOLERANCE),
            "a run starts at the label's response {label:?}, got {origins:?}"
        );
    }
}

/// A stack's children follow the response the row positions: the stack sits
/// on the leading edge, its first member (physically last in right-to-left)
/// touching it, centred vertically, and each label's run draws there.
fn row_positions_a_stack_at_the_leading_edge(env: Environment, direction: LayoutDirection) {
    let mut runtime = runtime(
        env,
        one_row_list(|| AnyView::new(hstack((text("One"), text("Two"))))),
    );
    let updates = settle(&mut runtime);
    let content = content_column(bounds(&updates, Role::ListItem, "One Two"));
    let (one, two) = (
        bounds(&updates, Role::Label, "One"),
        bounds(&updates, Role::Label, "Two"),
    );
    let stack = one.union(two);
    assert_leading(stack, content, direction);
    if direction.is_right_to_left() {
        assert_close(one.x1, content.x1, "first member's trailing edge");
    } else {
        assert_close(one.x0, content.x0, "first member's leading edge");
    }
    assert_runs_at(&runtime, &[one, two]);
}

#[test]
fn row_positions_a_stack_at_the_leading_edge_ltr() {
    row_positions_a_stack_at_the_leading_edge(test_environment(), LayoutDirection::LeftToRight);
}

#[test]
fn row_positions_a_stack_at_the_trailing_edge_rtl() {
    row_positions_a_stack_at_the_leading_edge(rtl_environment(), LayoutDirection::RightToLeft);
}

#[test]
fn row_positions_a_text_at_the_leading_edge() {
    let mut runtime = runtime(
        test_environment(),
        one_row_list(|| AnyView::new(text("Row"))),
    );
    let updates = settle(&mut runtime);
    let content = content_column(bounds(&updates, Role::ListItem, "Row"));
    let response = bounds(&updates, Role::Label, "Row");
    assert_leading(response, content, LayoutDirection::LeftToRight);
    assert_runs_at(&runtime, &[response]);
}

/// The content keeps the whole content column as its allocation, so a
/// geometry source naming it reports the column, not the positioned
/// response.
fn row_content_keeps_its_allocation(env: Environment) {
    let mut runtime = runtime(
        env,
        one_row_list(|| AnyView::new(text("Row").a11y_geometry(AccessibilityGeometry::Allocation))),
    );
    let updates = settle(&mut runtime);
    let content = content_column(bounds(&updates, Role::ListItem, "Row"));
    let allocation = bounds(&updates, Role::Label, "Row");
    assert_close(allocation.x0, content.x0, "allocation x0");
    assert_close(allocation.x1, content.x1, "allocation x1");
    assert_close(
        centre(allocation).1,
        centre(content).1,
        "allocation centre y",
    );
}

#[test]
fn row_content_keeps_its_allocation_ltr() {
    row_content_keeps_its_allocation(test_environment());
}

#[test]
fn row_content_keeps_its_allocation_rtl() {
    row_content_keeps_its_allocation(rtl_environment());
}
