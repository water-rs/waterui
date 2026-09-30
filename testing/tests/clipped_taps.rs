//! Coverage for water-rs/waterui#1325: `tap_at` resolves its point through
//! `HeadlessRuntime::accessibility_activation_point`, which projects the
//! node's interaction owner through its clip chain and the window. A row
//! straddling the scroll edge is tapped at its visible fragment, and a
//! fully clipped row fails with a not-visible panic instead of
//! dead-tapping inside the clip.

use std::time::Duration;

use waterui::Binding;
use waterui::Signal as _;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::{hstack, spacer, text};
use waterui::id::SelfId;
use waterui::reactive::collection::List as ReactiveList;
use waterui_testing::{Role, ui};

/// A 40 pt scroll leaves the first one-line row (56 pt under Material 3)
/// straddling the viewport's top edge with a 16 pt visible sliver — and
/// that sliver is padding only: the row's `on_tap` strip, centred inside
/// the row at content height, already lies fully above the viewport.
const FULL_SCROLL: f32 = 40.0;
/// A shallower scroll leaves the same row straddling with its `on_tap`
/// strip partially visible: the strip's own top is clipped away while its
/// lower span still crosses the viewport edge.
const PARTIAL_SCROLL: f32 = 28.0;
const ROW_COUNT: i32 = 10;

fn row_items() -> ReactiveList<SelfId<i32>> {
    ReactiveList::from((1..=ROW_COUNT).map(SelfId::new).collect::<Vec<_>>())
}

fn row_label(row: i32) -> String {
    format!("Row {row}")
}

/// Mounts a scrollable list whose rows count taps, scrolled by `scroll`,
/// and returns the mounted app, the row tap counter and the selection
/// binding — the view's evidence a tap reached the row's `on_tap`.
fn mount_scrolled_list(
    scroll: f32,
) -> (
    waterui_testing::OffscreenApp,
    Binding<i32>,
    Binding<Option<i32>>,
) {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let taps = Binding::container(0_i32);
    let counter = taps.clone();
    let mut app = ui()
        .theme(hydrolysis_m3::Material3::defaults())
        .viewport(360, 240)
        .mount_offscreen(move || {
            let counter = counter.clone();
            List::for_each(row_items(), move |item| {
                let row = item.into_inner();
                let counter = counter.clone();
                ListItem::new(hstack((text(row_label(row)), spacer())).on_tap(move || {
                    counter.with_mut(|c| *c += 1);
                }))
            })
            .selection(&binding)
        });
    app.settle();
    app.scroll_at(180.0, 120.0, 0.0, -scroll, false);
    app.pump_for(Duration::from_millis(500));
    (app, taps, selection)
}

/// The straddling precondition both tests share: row 1's logical bounds
/// cross the list's top clip edge.
fn assert_row_one_straddles(app: &mut waterui_testing::OffscreenApp) {
    let list = app.query().role(Role::LIST).single().bounds();
    let row = app
        .query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .single()
        .bounds();
    assert!(
        row.y() < list.y() && row.y() + row.height() > list.y(),
        "the straddling row reports its full logical bounds: row={row:?} list={list:?}"
    );
}

/// A `tap_at` on a row whose `on_tap` strip is only partly visible
/// resolves a point inside the surviving fragment, and a real pointer tap
/// there fires the row's action.
#[test]
fn tap_at_on_a_partially_straddling_row_fires_its_action() {
    let (mut app, taps, selection) = mount_scrolled_list(PARTIAL_SCROLL);
    assert_row_one_straddles(&mut app);

    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .tap_at(0.5, 0.5);
    assert_eq!(
        taps.snapshot(),
        1,
        "a tap at the resolved point on the visible strip fragment fires"
    );
    assert_eq!(selection.snapshot(), Some(1), "the row still selects");
}

/// At a deeper scroll the same row's `on_tap` strip lies fully above the
/// viewport, so `tap_at` has no reachable point: it panics naming the row
/// and reporting the target not visible rather than pressing inside the
/// clip or on the padding sliver the row's bounds still cover.
#[test]
#[should_panic(expected = "is not visible")]
fn tap_at_on_a_fully_clipped_row_fails_not_visible() {
    let (mut app, _taps, _selection) = mount_scrolled_list(FULL_SCROLL);
    assert_row_one_straddles(&mut app);

    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .tap_at(0.5, 0.5);
}
