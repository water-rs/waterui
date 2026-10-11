//! Layout regressions for the native navigation containers — the backend's
//! measure/proposal machinery, which stays here under water-rs/waterui#1673
//! mounted under `TestStyle`. The chrome-emission semantics moved to
//! `waterui-navigation`.
//!
//! <https://github.com/water-rs/hydrolysis/issues/153>: the split measured a
//! dynamically materialized column at intrinsic — `builder.build(selected)`
//! under a `ProposalSize` it never read — and the window's minimum-size probe
//! then resized the frame around an overflowing detail. Each column must
//! answer the proposal the split hands it, and measurement must read the
//! retained, mounted column rather than rebuilding it. `docs/layout-spec.md`
//! §7 is the contract: a column's rect is the proposal for its content.

mod support;

use accesskit::Rect;
use support::{mount_rendered, node_bounds_with_role};
use waterui::component::list::{List, ListItem};
use waterui::component::{hstack, text, vstack};
use waterui::id::SelfId;
use waterui::navigation::{NavigationSplitView, NavigationStack, NavigationView, Tab, Tabs};
use waterui::{Binding, View, ViewExt};
use waterui_core::Environment;

/// Widens a `u32` extent to `f32`, rounding to nearest when the value exceeds
/// the 24-bit mantissa; the extents involved are small.
#[expect(
    clippy::cast_precision_loss,
    reason = "the widened values are pixel extents, far below 2^24"
)]
const fn u32_as_f32(v: u32) -> f32 {
    v as f32
}

const WINDOW_WIDTH: u32 = 1400;
const WINDOW_HEIGHT: u32 = 900;
const BAR_HEIGHT: f32 = 48.0;

/// A lazy `List` of `count` text rows — the chat-log shape from the issue.
fn rows(count: usize, prefix: &'static str) -> impl View {
    let data: Vec<SelfId<usize>> = (0..count).map(SelfId::new).collect();
    List::for_each(data, move |item| {
        ListItem::new(text(format!("{prefix} {}", *item)))
    })
}

/// The `vstack` content of the detail pane: a lazy 60-row list and a
/// fixed-height trailing bar, with no spacing so the pane fills as
/// `list.height + bar.height` exactly.
fn detail_content() -> impl View {
    vstack((
        rows(60, "message").a11y_label("messages"),
        text("composer").height(BAR_HEIGHT).a11y_label("composer"),
    ))
    .spacing(0.0)
}

fn mount_split(selection: Binding<Option<i32>>) -> support::RenderedApp {
    mount_rendered(
        Environment::new(),
        move || {
            let selection = selection.clone();
            NavigationSplitView::new(
                &selection,
                move || rows(20, "chat").a11y_label("chats"),
                move |id| NavigationView::new(format!("Chat {id}"), detail_content()),
            )
        },
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
}

fn list_bounds(app: &mut support::RenderedApp, label: &'static str) -> Rect {
    node_bounds_with_role(app.tree(), accesskit::Role::List, label)
}

fn label_bounds(app: &mut support::RenderedApp, label: &'static str) -> Rect {
    node_bounds_with_role(app.tree(), accesskit::Role::Label, label)
}

fn assert_close(actual: f64, expected: f64, epsilon: f64, context: &str) {
    assert!(
        (actual - expected).abs() <= epsilon,
        "{context}: expected {expected}, measured {actual}"
    );
}

/// Asserts the geometry the issue fixes: the trailing bar inside the window,
/// the lazy list filling the pane minus the bar, and the sidebar's list height
/// unchanged by the detail materializing.
fn assert_split_geometry(app: &mut support::RenderedApp, context: &str) {
    let messages = list_bounds(app, "messages");
    let composer = label_bounds(app, "composer");
    assert!(
        composer.y0 + composer.height() <= f64::from(u32_as_f32(WINDOW_HEIGHT)) + 0.5,
        "{context}: composer escaped the window: {composer:?}"
    );
    // The pane is the detail column's content area: it starts where the list
    // starts and runs to the window bottom, and the bar closes it — so the
    // list's height is the pane minus the bar.
    let pane_height = f64::from(u32_as_f32(WINDOW_HEIGHT)) - messages.y0;
    assert_close(
        messages.height(),
        pane_height - f64::from(BAR_HEIGHT),
        1.0,
        context,
    );
}

#[test]
fn navigation_split_detail_fits_pane_when_selected_after_mount() {
    let selection = Binding::container(None::<i32>);
    let mut app = mount_split(selection.clone());
    app.settle();
    let sidebar_height_before = list_bounds(&mut app, "chats").height();

    selection.set(Some(7));
    app.settle();
    // A second selection forces the re-flush that, on the broken build,
    // replays the layout onto the surface the window just grew to — the
    // geometry assertions then observe the issue's overflow rather than a
    // stale pre-resize frame.
    selection.set(Some(8));
    app.settle();

    // The window itself must never have grown: the surface is the window.
    let snap = app.snapshot();
    assert_eq!(
        snap.height, WINDOW_HEIGHT,
        "the window grew to fit an overflowing detail column"
    );
    assert_split_geometry(&mut app, "selection set after mount");
    assert_close(
        list_bounds(&mut app, "chats").height(),
        sidebar_height_before,
        0.5,
        "the sidebar list's height changed when the detail appeared",
    );

    // The same geometry must result when the selection was already set at
    // mount — issue #153's before/after asymmetry.
    let mut before = mount_split(Binding::container(Some(7)));
    before.settle();
    let messages_after = list_bounds(&mut app, "messages");
    let composer_after = label_bounds(&mut app, "composer");
    let messages_before = list_bounds(&mut before, "messages");
    let composer_before = label_bounds(&mut before, "composer");
    for (after, before, name) in [
        (messages_after, messages_before, "message list"),
        (composer_after, composer_before, "composer"),
    ] {
        for (actual, expected, axis) in [
            (after.x0, before.x0, "x"),
            (after.y0, before.y0, "y"),
            (after.width(), before.width(), "width"),
            (after.height(), before.height(), "height"),
        ] {
            assert_close(
                actual,
                expected,
                0.5,
                &format!(
                    "{name} differs between selection-after-mount and selection-before-mount on {axis}"
                ),
            );
        }
    }
}

#[test]
fn navigation_split_detail_fits_pane_when_selected_before_mount() {
    let mut app = mount_split(Binding::container(Some(7)));
    app.settle();
    assert_split_geometry(&mut app, "selection set before mount");
}

/// The same ignored-proposal defect, in `Tabs`: a lazily measured tab content
/// reported its intrinsic height to its container's probe instead of the
/// content rect the tabs hands it, so the trailing sibling is pushed past the
/// window.
#[test]
fn tabs_content_fits_pane() {
    let selection = Binding::container(0i32);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            let selection = selection.clone();
            vstack((
                Tabs::new(
                    &selection,
                    vec![
                        Tab::new(0i32, "Messages", move || {
                            NavigationView::new(
                                "Messages",
                                rows(60, "message").a11y_label("messages"),
                            )
                        }),
                        Tab::new(1i32, "Settings", move || {
                            NavigationView::new("Settings", text("settings"))
                        }),
                    ],
                ),
                text("dock").height(BAR_HEIGHT).a11y_label("dock"),
            ))
            .spacing(0.0)
        },
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    );
    app.settle();

    let dock = label_bounds(&mut app, "dock");
    assert!(
        dock.y0 + dock.height() <= f64::from(u32_as_f32(WINDOW_HEIGHT)) + 0.5,
        "the vstack's trailing bar escaped the window: {dock:?}"
    );
    let messages = list_bounds(&mut app, "messages");
    // The tab content rect ends where the tab bar begins; the list inside
    // must fill it exactly.
    let tab_bar = support::nodes(app.tree())
        .find(|(_, node)| node.role() == accesskit::Role::TabList)
        .map(|(_, node)| node)
        .expect("the tabs emit a tab bar");
    let content_bottom = tab_bar.bounds().expect("the tab bar carries bounds").y0;
    assert_close(
        messages.y0 + messages.height(),
        content_bottom,
        1.0,
        "the tab content's list must fill the pane minus the dock",
    );
}

/// water-rs/waterui#1915, the measure-time half: a retained sub-view's
/// layout pass also reads text metrics — a `text!` binding inside
/// navigation content — through the `&mut HydroState` measure path rather
/// than `read_signal`, so no watch registered it: growing the bound text
/// must still re-run the sub-view's layout and shift the sibling beside it.
#[test]
fn navigation_content_relayouts_when_a_measure_read_text_grows() {
    let value = Binding::container(String::from("9"));
    let value_for_view = value.clone();
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            let value = value_for_view.clone();
            NavigationStack::new(NavigationView::new(
                "Inbox",
                hstack((
                    waterui::text!("{value}").a11y_label("counter"),
                    text("next").a11y_label("next"),
                ))
                .spacing(0.0),
            ))
        },
        320,
        240,
    );
    app.settle();
    let counter_before = label_bounds(&mut app, "counter");
    let next_before = label_bounds(&mut app, "next");

    value.set(String::from("10000"));
    app.settle();

    let counter_after = label_bounds(&mut app, "counter");
    let next_after = label_bounds(&mut app, "next");
    assert!(
        counter_after.width() > counter_before.width(),
        "the counter's laid-out frame did not grow with its text: \
         {counter_before:?} -> {counter_after:?}"
    );
    assert!(
        next_after.x0 > next_before.x0,
        "the sibling did not move when the text beside it grew: \
         {next_before:?} -> {next_after:?}"
    );
    // Zero spacing places the sibling exactly at the text's trailing edge,
    // wherever the row itself sits inside the page.
    assert_close(
        next_after.x0,
        counter_after.x0 + counter_after.width(),
        1.0,
        "the sibling must start at the grown text's trailing edge",
    );
}
