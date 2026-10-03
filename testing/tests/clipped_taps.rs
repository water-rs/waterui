//! Coverage for water-rs/waterui#1325: `tap_at` resolves its point through
//! `HeadlessRuntime::accessibility_activation_point`, which projects the
//! node's interaction owner through its clip chain and the window. A row
//! straddling the scroll edge is tapped at its visible fragment, and a
//! fully clipped row fails with a not-visible panic instead of
//! dead-tapping inside the clip.
//!
//! The scroll distances are derived from geometry measured in the mounted
//! tree — the row's bounds and the `on_tap` strip's resolved edges — never
//! from the theme's row metrics.

use std::time::Duration;

use waterui::Binding;
use waterui::Signal as _;
use waterui::ViewExt as _;
use waterui::component::list::{List, ListItem};
use waterui::component::{hstack, spacer, text};
use waterui::id::SelfId;
use waterui::reactive::collection::List as ReactiveList;
use waterui_testing::{
    AccessibilityActivationPointError, ElementRef, HeadlessRuntime, NodeId, OffscreenApp, Role, ui,
};

const ROW_COUNT: i32 = 10;

fn row_items() -> ReactiveList<SelfId<i32>> {
    ReactiveList::from((1..=ROW_COUNT).map(SelfId::new).collect::<Vec<_>>())
}

fn row_label(row: i32) -> String {
    format!("Row {row}")
}

/// Mounts a scrollable list whose rows count taps and returns the mounted
/// app, the row tap counter and the selection binding — the view's
/// evidence a tap reached the row's `on_tap`.
fn mount_list() -> (OffscreenApp, Binding<i32>, Binding<Option<i32>>) {
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
    (app, taps, selection)
}

/// The straddling row the scroll distances derive from.
fn row_one(app: &mut OffscreenApp) -> ElementRef<HeadlessRuntime> {
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .single()
}

/// The list's top clip edge, in the same window coordinates the nodes
/// report their bounds in.
fn clip_top(app: &mut OffscreenApp) -> f32 {
    app.query().role(Role::LIST).single().bounds().y()
}

/// The vertical span of the row's `on_tap` strip. The strip is not its
/// own accessibility node — it is the interaction owner the row's `Click`
/// delegates to — so its edges are measured through `activation_point`,
/// the projection that resolves against exactly that region: the `0.0`
/// and `1.0` vertical fractions are the strip's top and bottom edges.
fn strip_edges(app: &OffscreenApp, row: NodeId) -> (f32, f32) {
    let top = app
        .activation_point(row, 0.5, 0.0)
        .expect("a mounted row's strip resolves a hittable point")
        .1;
    let bottom = app
        .activation_point(row, 0.5, 1.0)
        .expect("a mounted row's strip resolves a hittable point")
        .1;
    (top, bottom)
}

/// Scrolls the list up by `distance` logical points and lets the scroll
/// glide come to rest.
fn scroll_by(app: &mut OffscreenApp, distance: f32) {
    app.scroll_at(180.0, 120.0, 0.0, -distance, false);
    app.pump_for(Duration::from_millis(500));
}

/// The straddling precondition both tests share: row 1's logical bounds
/// cross the list's top clip edge.
fn assert_row_one_straddles(app: &mut OffscreenApp) {
    let list = app.query().role(Role::LIST).single().bounds();
    let row = row_one(app).bounds();
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
    let (mut app, taps, selection) = mount_list();
    let clip = clip_top(&mut app);
    let row = row_one(&mut app);
    let (strip_top, strip_bottom) = strip_edges(&app, row.id());

    // Put the strip's midpoint exactly on the clip edge: its top half is
    // clipped while the lower half stays visible — the partial straddle.
    scroll_by(&mut app, strip_top.midpoint(strip_bottom) - clip);
    assert_row_one_straddles(&mut app);
    let top_edge = app
        .activation_point(row.id(), 0.5, 0.0)
        .expect("a partially visible strip still resolves a point")
        .1;
    assert!(
        top_edge <= clip + 1.0,
        "the strip's clipped top edge resolves at the clip: edge={top_edge} clip={clip}"
    );

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
    let (mut app, _taps, _selection) = mount_list();
    let clip = clip_top(&mut app);
    let row = row_one(&mut app);
    let row_bottom = row.bounds().y() + row.bounds().height();
    let (_strip_top, strip_bottom) = strip_edges(&app, row.id());

    // Put the midpoint of the row's bottom padding on the clip edge: the
    // whole strip is above the viewport while half the padding still
    // straddles it.
    scroll_by(&mut app, strip_bottom.midpoint(row_bottom) - clip);
    assert_row_one_straddles(&mut app);
    assert_eq!(
        app.activation_point(row.id(), 0.5, 0.5),
        Err(AccessibilityActivationPointError::EmptyFragment),
        "the fully clipped strip leaves no pointer-reachable fragment"
    );

    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .tap_at(0.5, 0.5);
}
