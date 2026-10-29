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
use support::{mount_rendered, node_bounds_with_role, node_label_exists};
use waterui::component::list::{List, ListItem};
use waterui::component::{hstack, text, vstack};
use waterui::id::SelfId;
use waterui::navigation::{NavigationSplitView, NavigationStack, NavigationView, Tab, Tabs};
use waterui::{Binding, View, ViewExt};
use waterui_controls::label::{LabelDisplayMode, label};
use waterui_core::{AnyView, Environment};

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
    let selection = Binding::container(0usize);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            let selection = selection.clone();
            vstack((
                Tabs::new(
                    &selection,
                    vec![
                        Tab::new("Messages", move || {
                            NavigationView::new(
                                "Messages",
                                rows(60, "message").a11y_label("messages"),
                            )
                        }),
                        Tab::new("Settings", move || {
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

/// water-rs/waterui#1939: a pushed page's back button owns the bar's
/// leading edge, so its leading toolbar group must start at the button's
/// trailing edge — the issue had a `Cancellation` item drawing under the
/// button, "the arrow covers the first letters of Cancel".
#[test]
fn a_pushed_pages_cancellation_item_clears_the_back_button() {
    let path = waterui::navigation::NavigationPath::<u8>::new();
    path.push(1);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            NavigationStack::with_path(path.clone(), NavigationView::new("Root", text("root")))
                .destination(|_| {
                    NavigationView::new("Detail", text("detail")).navigation_toolbar(
                        waterui::navigation::NavigationToolbar::default().item(
                            waterui::navigation::NavigationToolbarItem::new(
                                waterui::navigation::NavigationToolbarPlacement::Cancellation,
                                waterui::component::button("Cancel").action(|| {}),
                            ),
                        ),
                    )
                })
        },
        320,
        240,
    );
    app.settle();

    let back = node_bounds_with_role(app.tree(), accesskit::Role::Button, "Back");
    let cancel = node_bounds_with_role(app.tree(), accesskit::Role::Button, "Cancel");
    assert!(
        cancel.x0 >= back.x1 - 1.0,
        "the cancellation item must start at the back button's trailing edge: \
         back {back:?}, cancel {cancel:?}"
    );
}

/// water-rs/waterui#3710: a bar action whose label has an icon shows the
/// icon alone unless the caller chose a display mode, and keeps its title as
/// the accessible name.
#[test]
fn a_trailing_bar_action_with_an_icon_label_is_icon_only() {
    // Long enough to exceed the test style's minimum button width, so the
    // title's presence shows in the button's width.
    const TITLE: &str = "Delete every selected item permanently";
    let delete_button_width = |mode: Option<LabelDisplayMode>| {
        let mut app = mount_rendered(
            Environment::new(),
            move || {
                let button = waterui::component::button(label(TITLE).icon(text("x")));
                let button = match mode {
                    Some(mode) => AnyView::new(button.label_style(mode).action(|| {})),
                    None => AnyView::new(button.action(|| {})),
                };
                NavigationView::new("Root", text("root")).navigation_toolbar(
                    waterui::navigation::NavigationToolbar::default().item(
                        waterui::navigation::NavigationToolbarItem::new(
                            waterui::navigation::NavigationToolbarPlacement::TopBarTrailing,
                            button,
                        ),
                    ),
                )
            },
            320,
            240,
        );
        app.settle();
        let bounds = node_bounds_with_role(app.tree(), accesskit::Role::Button, TITLE);
        bounds.x1 - bounds.x0
    };

    let in_bar = delete_button_width(None);
    let explicit = delete_button_width(Some(LabelDisplayMode::TitleAndIcon));
    assert!(
        in_bar < explicit,
        "the bar action must draw the icon alone ({in_bar}) while an explicit \
         title-and-icon mode keeps both ({explicit})"
    );
}

// ---------------------------------------------------------------------------
// Reactive-collection `Tabs` — water-rs/waterui#1194
//
// `Tabs` over `nami::collection::List<SelfId<i32>>`: a tab's `SelfId` is its
// identity, a surviving id keeps its retained subviews — content, navigation
// state — and only inserted ids materialize.
// ---------------------------------------------------------------------------

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use waterui::reactive::Signal as _;
use waterui::reactive::collection::List as SignalList;
use waterui::views::ForEach;

/// The tabs fixture's collection handle, kept mutable outside the app.
type TabIds = SignalList<SelfId<i32>>;

fn keyed_tabs(ids: Vec<i32>) -> (TabIds, Vec<SelfId<i32>>) {
    let panes: Vec<SelfId<i32>> = ids.into_iter().map(SelfId::new).collect();
    (SignalList::from(panes.clone()), panes)
}

/// Mounts a `Tabs` whose tabs are keyed by `list`'s `SelfId`s; each tab's
/// content builder counts its invocations into `builds[i]` so a rebuild of a
/// surviving tab shows up as an extra count.
fn mount_keyed(
    list: &TabIds,
    labels: &'static [&'static str],
    builds: &[Rc<Cell<usize>>],
    selection: Binding<i32>,
) -> support::RenderedApp {
    let list = list.clone();
    let labels = labels.to_vec();
    let builds = builds.to_vec();
    mount_rendered(
        Environment::new(),
        move || {
            let selection = selection.clone();
            Tabs::new(
                &selection,
                ForEach::new(list, move |pane: SelfId<i32>| {
                    let index = usize::try_from(*pane).expect("fixture ids index the fixtures");
                    let builds = builds[index].clone();
                    let label = labels[index];
                    Tab::new(label, move || {
                        builds.set(builds.get() + 1);
                        NavigationView::new(label, text(label))
                    })
                }),
            )
        },
        WINDOW_WIDTH,
        WINDOW_HEIGHT,
    )
}

#[test]
fn an_empty_tab_set_mounts() {
    let (list, _panes) = keyed_tabs(vec![]);
    let selection = Binding::container(0i32);
    let mut app = mount_keyed(&list, &[], &[], selection);
    app.settle();
    // The container mounts — the pump did not assert — and no tab chrome or
    // selected content reaches the accessibility tree.
    assert!(!node_label_exists(app.tree(), "tab"));
}

#[test]
fn inserting_a_tab_keeps_existing_tabs_retained() {
    let builds = vec![
        Rc::new(Cell::new(0usize)),
        Rc::new(Cell::new(0usize)),
        Rc::new(Cell::new(0usize)),
    ];
    let (mut list, _panes) = keyed_tabs(vec![0, 1]);
    let selection = Binding::container(0i32);
    let mut app = mount_keyed(&list, &["Tab A", "Tab B", "Tab C"], &builds, selection);
    app.settle();
    assert_eq!(builds[0].get(), 1);
    assert_eq!(builds[1].get(), 1);

    list.push(SelfId::new(2));
    app.pump();

    // The inserted tab materialized once; the surviving tabs were not
    // rebuilt.
    assert_eq!(builds[2].get(), 1);
    assert_eq!(builds[0].get(), 1);
    assert_eq!(builds[1].get(), 1);
    assert!(node_label_exists(app.tree(), "Tab C"));
}

#[test]
fn removing_the_selected_tab_relocates_selection() {
    let builds: Vec<_> = (0..3).map(|_| Rc::new(Cell::new(0usize))).collect();
    let (mut list, _panes) = keyed_tabs(vec![0, 1, 2]);
    let selection = Binding::container(2i32); // trailing tab selected
    let mut app = mount_keyed(
        &list,
        &["Tab A", "Tab B", "Tab C"],
        &builds,
        selection.clone(),
    );
    app.settle();

    // Removing the trailing selected tab relocates the binding to the new
    // last tab — the tab now at the removed tab's index, clamped.
    let removed = list.remove(2);
    assert_eq!(*removed, 2);
    app.pump();
    assert_eq!(
        selection.snapshot(),
        1,
        "the selection relocates to the new last tab"
    );
}

#[test]
fn reordering_tabs_keeps_their_state() {
    let builds: Vec<_> = (0..3).map(|_| Rc::new(Cell::new(0usize))).collect();
    let (mut list, _panes) = keyed_tabs(vec![0, 1, 2]);
    let selection = Binding::container(2i32);
    let mut app = mount_keyed(
        &list,
        &["Tab A", "Tab B", "Tab C"],
        &builds,
        selection.clone(),
    );
    app.settle();

    // Move the trailing tab to the front: a remove + insert where every id
    // survives — no content rebuilds and the selection still names tab 2.
    let moved = list.remove(2);
    list.insert(0, moved);
    app.pump();

    assert!(builds.iter().all(|builds| builds.get() == 1));
    assert_eq!(selection.snapshot(), 2);
    assert!(node_label_exists(app.tree(), "Tab C"));
}

#[test]
fn removing_every_tab_leaves_the_selection_untouched() {
    let builds: Vec<_> = (0..2).map(|_| Rc::new(Cell::new(0usize))).collect();
    let (mut list, _panes) = keyed_tabs(vec![0, 1]);
    let selection = Binding::container(1i32);
    let mut app = mount_keyed(&list, &["Tab A", "Tab B"], &builds, selection.clone());
    app.settle();

    list.clear();
    app.pump();

    // Empty set: nothing selected, the binding is left untouched and the
    // container still mounts — the pump did not assert.
    assert_eq!(selection.snapshot(), 1);
    assert!(!node_label_exists(app.tree(), "Tab A"));
    assert!(!node_label_exists(app.tree(), "Tab B"));
}

#[test]
fn reinserting_the_selected_tab_after_clearing_restores_it() {
    // Clearing every tab leaves the binding untouched (the empty set has no
    // selection); reinserting the same ids must find that binding again —
    // the retained tab mounts its content, not a relocated selection.
    let builds: Vec<_> = (0..3).map(|_| Rc::new(Cell::new(0usize))).collect();
    let (mut list, panes) = keyed_tabs(vec![0, 1, 2]);
    let selection = Binding::container(2i32);
    let mut app = mount_keyed(
        &list,
        &["Tab A", "Tab B", "Tab C"],
        &builds,
        selection.clone(),
    );
    app.settle();

    list.clear();
    app.pump();
    assert_eq!(selection.snapshot(), 2);
    assert!(!node_label_exists(app.tree(), "Tab C"));

    // Reinsert the same membership — the binding still names tab id 2, the
    // reconcile finds it and shows its content without a relocation write.
    for pane in panes {
        list.push(pane);
    }
    app.pump();

    assert_eq!(selection.snapshot(), 2);
    assert!(
        node_label_exists(app.tree(), "Tab C"),
        "reinserting the selected id restores its pane"
    );
}

#[test]
fn a_selection_watcher_mutating_the_collection_does_not_deadlock() {
    // The relocation write runs watchers synchronously. A watcher that
    // mutates the tab collection re-enters the membership watch — the
    // render state must not be borrowed while the write publishes.
    let builds: Vec<_> = (0..2).map(|_| Rc::new(Cell::new(0usize))).collect();
    let (mut list, _panes) = keyed_tabs(vec![0, 1]);
    let selection = Binding::container(0i32);
    let mut app = mount_keyed(&list, &["Tab A", "Tab B"], &builds, selection.clone());
    app.settle();

    let list_in_watcher = RefCell::new(list.clone());
    let guard = selection.watch(move |_| {
        list_in_watcher.borrow_mut().push(SelfId::new(0));
    });

    // Removing the selected tab relocates the binding → the watcher
    // re-inserts the removed tab while the membership watch is mid-publish.
    let removed = list.remove(0);
    assert_eq!(*removed, 0);
    app.pump();

    assert_eq!(
        selection.snapshot(),
        1,
        "the relocation publishes the tab at the removed index"
    );
    assert!(node_label_exists(app.tree(), "Tab B"));
    app.pump();
    assert!(
        node_label_exists(app.tree(), "Tab A"),
        "the watcher's insertion lands on the next pass"
    );
    drop(guard);
}
