//! List widget regressions.

//! <https://github.com/water-rs/hydrolysis/issues/168>: `apply_scroll_request`
//! asserted `index < row_count`, but the row count comes from signal-driven
//! contents — a `List` materializing mid-flush with a pending target above its
//! current row count panicked inside `list_accessibility`. The watergram
//! dogfood hits it on every chat open: the message list clears to 0 rows while
//! the previous contents' target is still pending. A scroll request names a
//! row that may not exist yet: it stays pending until the contents contain the
//! index, and a newer generation supersedes it.

use std::cell::Cell;
use std::rc::Rc;

use hydrolysis_m3::Material3;
use nami::collection::List as ReactiveList;
use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::component::{hstack, spacer, text};
use waterui::id::SelfId;
use waterui::layout::scroll::ScrollController;
use waterui::{Binding, View, ViewExt};
use waterui_core::dynamic::watch;
use waterui_testing::{Role, ui};

/// Material's one-line list row: a scroll request landing on row `N` reports a
/// `scroll_y` of `N * ROW_HEIGHT` on the rendered runtime.
const ROW_HEIGHT: f64 = 56.0;
/// The pending target from the issue — a row the mounted contents do not have.
const SCROLL_TARGET: usize = 8;
/// The contents grow past the target after mount, so the pending request
/// becomes actionable.
const GROWN_ROW_COUNT: usize = 20;

/// The lazy identity-keyed list from the issue, with its scroll controller
/// attached and an accessibility label to query it by.
fn pending_scroll_list(
    items: ReactiveList<SelfId<usize>>,
    controller: ScrollController<usize>,
) -> impl View {
    List::for_each(items, |item| ListItem::new(text(format!("row {}", *item))))
        .scroll_controller(&controller)
        .a11y_label("messages")
}

/// The semantic runtime runs `list_accessibility` with no render context —
/// its scroll domain is row units, so a landed request reports the target
/// index as `scroll_y`.
#[test]
fn pending_scroll_target_above_row_count_waits_for_contents_semantic() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let controller = ScrollController::new(0);
    // The request lands while the contents are still empty: the dogfood's
    // cleared message list carried a pending target in exactly this shape.
    controller.scroll_to(SCROLL_TARGET);
    let mut app = ui().mount({
        let items = items.clone();
        let controller = controller.clone();
        move || pending_scroll_list(items.clone(), controller.clone())
    });
    // Growing the contents past the target makes the pending request
    // actionable; it must not have been dropped while unreachable.
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    app.settle();
    let scroll_y = app
        .query()
        .role(Role::LIST)
        .label("messages")
        .single()
        .node()
        .scroll_y()
        .expect("the list reports a scroll offset");
    assert!(
        (scroll_y - SCROLL_TARGET as f64).abs() < 0.5,
        "pending scroll should land on row {SCROLL_TARGET} once the contents reach it: scroll_y={scroll_y}"
    );
}

/// The rendered runtime takes the same `apply_scroll_request` path through
/// `render_list`; its scroll domain is points, and the virtualized window
/// shows which rows are realized.
#[test]
fn pending_scroll_target_above_row_count_waits_for_contents_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let controller = ScrollController::new(0);
    controller.scroll_to(SCROLL_TARGET);
    let mut app = ui()
        .viewport(320, 320)
        .theme(Material3::defaults())
        .mount_offscreen({
            let items = items.clone();
            let controller = controller.clone();
            move || pending_scroll_list(items.clone(), controller.clone())
        });
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    app.settle();
    let scroll_y = app
        .query()
        .role(Role::LIST)
        .label("messages")
        .single()
        .node()
        .scroll_y()
        .expect("the list reports a scroll offset");
    let expected = SCROLL_TARGET as f64 * ROW_HEIGHT;
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "pending scroll should land on row {SCROLL_TARGET}: expected scroll_y≈{expected}, got {scroll_y}"
    );
    // The landed scroll moved the virtualized window: the target row is
    // realized, row 0 scrolled out and was evicted.
    app.query().label("row 8").assert_exists();
    app.query().label("row 0").assert_not_exists();
}

/// <https://github.com/water-rs/hydrolysis/issues/111>: a tap on `List` row
/// content never fires once the row's content re-renders while the row's swipe
/// gesture stays armed. The retained swipe re-registers with the hit-test order
/// minted on its birth frame while the rebuilt tap mints a fresh order — and
/// the order counter resets every rebuild, so the stale order can permanently
/// outrank the tap: `top_group_id_at` then picks the swipe's gesture group and
/// the tap recognizer never activates.
#[test]
fn row_content_tap_survives_retained_swipe_order_offscreen() {
    let highlight = Binding::bool(false);
    let taps = Rc::new(Cell::new(0));
    let mut app = ui()
        .viewport(360, 240)
        .theme(Material3::defaults())
        .mount_offscreen({
            let taps = Rc::clone(&taps);
            let highlight = highlight.clone();
            move || {
                let highlight = highlight.clone();
                let taps = Rc::clone(&taps);
                let items =
                    ReactiveList::from((1..=3).map(SelfId::new).collect::<Vec<SelfId<i32>>>());
                List::for_each(items, move |item| {
                    let row = *item;
                    let taps = Rc::clone(&taps);
                    ListItem::new(watch(highlight.clone(), move |on| {
                        let label = if on { "high" } else { "low" };
                        let taps = Rc::clone(&taps);
                        hstack((text(format!("row {row} {label}")), spacer()))
                            .on_tap(move || taps.set(row))
                    }))
                })
                .on_delete(|_: ListDelete| {})
            }
        });

    app.query().label_contains("row 2").tap_at(0.5, 0.5);
    assert_eq!(taps.get(), 2, "baseline: row content on_tap must fire");

    // Rebuilding the row contents re-mints their tap targets while the rows'
    // swipe gestures stay retained; the taps must still outrank them.
    highlight.set(true);
    app.settle();

    taps.set(0);
    app.query().label_contains("row 2").tap_at(0.5, 0.5);
    assert_eq!(
        taps.get(),
        2,
        "row content on_tap must still fire after the contents rebuild"
    );
}

/// <https://github.com/water-rs/hydrolysis/issues/52>: a `List` in edit mode
/// draws a delete control and a reorder handle on every row but published no
/// accessibility node for either — the tree was byte-identical to the same
/// list without `editing`, `on_delete` or `on_move`, so no assistive
/// technology (and no `Query::tap`, which acts on nodes) could reach them.
/// Each control now emits a `Button` node at the drawn control bounds whose
/// `Click` runs the same handler its pointer target does.
#[test]
fn edit_controls_emit_accessibility_nodes_offscreen() {
    let deletes = Rc::new(Cell::new(usize::MAX));
    let moves = Rc::new(Cell::new((usize::MAX, usize::MAX)));
    let mut app = ui()
        .viewport(360, 240)
        .theme(Material3::defaults())
        .mount_offscreen({
            let deletes = Rc::clone(&deletes);
            let moves = Rc::clone(&moves);
            move || {
                let deletes = Rc::clone(&deletes);
                let moves = Rc::clone(&moves);
                let items =
                    ReactiveList::from((1..=3).map(SelfId::new).collect::<Vec<SelfId<i32>>>());
                List::for_each(items, move |item| {
                    ListItem::new(text(format!("row {}", *item)))
                })
                .editing(true)
                .on_delete(move |ListDelete(index)| deletes.set(index))
                .on_move(move |ListMove(reorder)| moves.set((reorder.from(), reorder.to())))
            }
        });

    // Three editing rows emit three delete controls; the reorder handle's up
    // half exists only where a row can move up, its down half where it can
    // move down — two each across three rows.
    assert_eq!(
        app.query().role(Role::BUTTON).label("Delete").all().len(),
        3,
        "every editing row's delete control"
    );
    assert_eq!(
        app.query().role(Role::BUTTON).label("Move up").all().len(),
        2,
        "every movable row's move-up control"
    );

    // `tap` performs the node's `Click` — it must reach the same handler the
    // drawn control's pointer target runs. Handles are re-resolved right
    // before acting: every query settles a fresh tree revision.
    let delete_nodes = app.query().role(Role::BUTTON).label("Delete").all();
    delete_nodes[1].tap(&mut app);
    assert_eq!(
        deletes.get(),
        1,
        "tapping row 1's delete control deletes it"
    );

    let move_downs = app.query().role(Role::BUTTON).label("Move down").all();
    move_downs[0].tap(&mut app);
    assert_eq!(
        moves.get(),
        (0, 1),
        "row 0's move-down control moves it down"
    );
}

/// An item whose id stays stable while a baked field changes — the shape from
/// <https://github.com/water-rs/hydrolysis/issues/227>: `get_id` still resolves
/// the same identity, so membership reconciliation alone never sees that the
/// row's retained content is stale.
#[derive(Clone)]
struct FieldRow {
    id: u64,
    badge: &'static str,
}

impl waterui::id::Identifiable for FieldRow {
    type Id = u64;

    fn id(&self) -> Self::Id {
        self.id
    }
}

fn badge_list(items: nami::collection::SignalCollection<Binding<Vec<FieldRow>>>) -> impl View {
    List::for_each(items, |item| {
        ListItem::new(waterui::component::vstack((
            text(format!("row {}", item.id)),
            text(item.badge),
        )))
    })
}

/// The semantic path: a same-id update must re-materialize the retained row so
/// the row's *emitted descendants* carry the new fields. The row's own label is
/// re-read from a fresh materialization every emit, so only a descendant leaf
/// can observe whether the retained subtree rebuilt.
#[test]
fn for_each_same_id_item_update_rematerializes_row_semantic() {
    let items = Binding::container(vec![
        FieldRow {
            id: 1,
            badge: "badge-a",
        },
        FieldRow {
            id: 2,
            badge: "badge-b",
        },
    ]);
    let mut app = ui().mount({
        let items = items.clone();
        move || badge_list(nami::collection::SignalCollection::new(items.clone()))
    });
    app.settle();
    app.query()
        .role(Role::LABEL)
        .label("badge-a")
        .assert_exists();

    items.set(vec![
        FieldRow {
            id: 1,
            badge: "badge-z",
        },
        FieldRow {
            id: 2,
            badge: "badge-b",
        },
    ]);
    app.settle();

    app.query()
        .role(Role::LABEL)
        .label("badge-z")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("badge-a")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label("badge-b")
        .assert_exists();
}

/// The rendered path: the same update must repaint the changed row, and the
/// virtualized row bookkeeping keyed by item id — scroll offset and row focus —
/// survives the re-materialization.
#[test]
fn for_each_same_id_item_update_rematerializes_row_offscreen() {
    let items = Binding::container(
        (0..10)
            .map(|id| FieldRow {
                id,
                badge: "badge-a",
            })
            .collect::<Vec<_>>(),
    );
    let mut app = ui()
        .viewport(320, 320)
        .theme(Material3::defaults())
        .mount_offscreen({
            let items = items.clone();
            move || badge_list(nami::collection::SignalCollection::new(items.clone()))
        });
    app.settle();
    let before = app.snapshot();

    // Focus a row so its claim is retained by id, then update a different row.
    app.tap_at(20.0, 20.0);
    app.settle();

    items.set(
        (0..10)
            .map(|id| FieldRow {
                id,
                badge: if id == 1 { "badge-z" } else { "badge-a" },
            })
            .collect::<Vec<_>>(),
    );
    app.settle();
    let after = app.snapshot();
    assert!(
        after.rgba8 != before.rgba8,
        "the updated row must repaint: a same-id content change must re-materialize \
         the retained row instead of replaying the stale subtree"
    );
    app.query()
        .role(Role::LABEL)
        .label("badge-z")
        .assert_exists();
}
