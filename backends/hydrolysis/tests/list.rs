//! List widget regressions.

//! <https://github.com/water-rs/hydrolysis/issues/168>: `apply_scroll_request`
//! asserted `index < row_count`, but the row count comes from signal-driven
//! contents — a `List` materializing mid-flush with a pending target above its
//! current row count panicked inside `list_accessibility`. The watergram
//! dogfood hits it on every chat open: the message list clears to 0 rows while
//! the previous contents' target is still pending. A scroll request names a
//! row that may not exist yet: it stays pending until the contents contain the
//! index, and a newer generation supersedes it.

mod support {
    /// Widens a `usize` count to `f64`, rounding to nearest.
    #[expect(clippy::cast_precision_loss, reason = "test counts are far below 2^53")]
    pub const fn usize_as_f64(v: usize) -> f64 {
        v as f64
    }
}

use std::cell::Cell;
use std::rc::Rc;

use hydrolysis_m3::Material3;
use nami::collection::List as ReactiveList;
use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::component::{hstack, spacer, text, vstack};
use waterui::id::SelfId;
use waterui::layout::scroll::ScrollController;
use waterui::{Binding, View, ViewExt};
use waterui_core::dynamic::watch;
use waterui_testing::{DragOptions, OffscreenApp, Role, RuntimeDriver, SemanticApp, ui};

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
#[allow(clippy::needless_pass_by_value)]
fn pending_scroll_list(
    items: ReactiveList<SelfId<usize>>,
    controller: ScrollController<usize>,
) -> impl View {
    List::for_each(items, |item| ListItem::new(text(format!("row {}", *item))))
        .scroll_controller(&controller)
        .a11y_label("messages")
}

/// Mounts `view` offscreen at the 320pt viewport the list tests share and
/// settles the first frames.
fn mount_list<V: View>(view: impl Fn() -> V + 'static) -> OffscreenApp {
    let mut app = ui()
        .viewport(320, 320)
        .theme(Material3::defaults())
        .mount_offscreen(view);
    app.settle();
    app
}

/// Mounts [`pending_scroll_list`] over `items`, driven by `controller`.
fn mount_messages(
    items: &ReactiveList<SelfId<usize>>,
    controller: &ScrollController<usize>,
) -> OffscreenApp {
    let items = items.clone();
    let controller = controller.clone();
    mount_list(move || pending_scroll_list(items.clone(), controller.clone()))
}

/// The `messages` offset in the tree the last pumped frame published, read
/// without the query's sync pumps: a query first applies whatever that frame
/// left pending — a membership re-anchor, rows measured late — so a
/// frame-exact sample of what one frame published reads the held tree
/// directly.
fn frame_scroll_y(app: &OffscreenApp) -> f64 {
    app.tree()
        .nodes()
        .values()
        .find(|node| node.role() == Role::LIST && node.label() == Some("messages"))
        .expect("the frame published the messages list")
        .scroll_y()
        .expect("the list reports a scroll offset")
}

/// The `messages` list's reported vertical offset: points offscreen, row
/// units on the semantic runtime.
fn scroll_y<R: RuntimeDriver>(app: &mut SemanticApp<R>) -> f64 {
    app.query()
        .role(Role::LIST)
        .label("messages")
        .single()
        .node()
        .scroll_y()
        .expect("the list reports a scroll offset")
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
        let controller = controller;
        move || pending_scroll_list(items.clone(), controller.clone())
    });
    // Growing the contents past the target makes the pending request
    // actionable; it must not have been dropped while unreachable.
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    app.settle();
    let scroll_y = scroll_y(&mut app);
    assert!(
        (scroll_y - support::usize_as_f64(SCROLL_TARGET)).abs() < 0.5,
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
    let mut app = mount_messages(&items, &controller);
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    app.settle();
    let scroll_y = scroll_y(&mut app);
    let expected = support::usize_as_f64(SCROLL_TARGET) * ROW_HEIGHT;
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "pending scroll should land on row {SCROLL_TARGET}: expected scroll_y≈{expected}, got {scroll_y}"
    );
    // The landed scroll moved the virtualized window: the target row is
    // realized, row 0 scrolled out and was evicted.
    app.query().label("row 8").assert_exists();
    app.query().label("row 0").assert_not_exists();
}

/// An `animate_to` request rides the request's curve to the row and lands
/// exactly where `scroll_to` would — the row's measured offset
/// (water-rs/waterui#1901). Mid-flight the reported `scroll_y` is strictly
/// between the two requests' rest and landing points.
#[test]
fn animate_to_a_row_glides_and_lands_where_the_jump_would_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    // The jump lands on the row's offset immediately; it is the baseline the
    // animated request must converge to.
    controller.scroll_to(SCROLL_TARGET);
    app.settle();
    let jump_offset = scroll_y(&mut app);
    let expected = support::usize_as_f64(SCROLL_TARGET) * ROW_HEIGHT;
    assert!(
        (jump_offset - expected).abs() < 1.0,
        "jump should land on row {SCROLL_TARGET}'s offset ≈{expected}: {jump_offset}"
    );

    controller.scroll_to(0);
    app.settle();

    controller.animate_to(SCROLL_TARGET, waterui::animation::Animation::default());
    app.pump_for(std::time::Duration::from_millis(100));
    let midway = scroll_y(&mut app);
    assert!(
        midway > 0.0 && midway < jump_offset,
        "animated list scroll should be in flight mid-duration: scroll_y={midway}"
    );

    app.settle();
    // An animated list scroll must land on the same offset as the jump.
    approx::assert_relative_eq!(scroll_y(&mut app), jump_offset);
    app.query().label("row 8").assert_exists();
    app.query().label("row 0").assert_not_exists();
}

/// Reading the tree never advances the clock (water-rs/waterui#2243): a
/// query between two frames of an `animate_to` run answers the frame the
/// clock is on, so sampling the run once per frame observes one frame of
/// motion per sample — the whole glide, not a jump to its end after a
/// sample or two.
#[test]
fn querying_between_frames_of_animate_to_observes_every_frame_offscreen() {
    /// A linear run moves the same distance every frame, so a sample that
    /// moved further than that saw the clock advance behind its back.
    const DURATION: std::time::Duration = std::time::Duration::from_millis(320);
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.animate_to(
        SCROLL_TARGET,
        waterui::animation::Animation::linear(DURATION),
    );
    let target = support::usize_as_f64(SCROLL_TARGET) * ROW_HEIGHT;
    let frames = DURATION.as_millis() / waterui_testing::VIRTUAL_FRAME.as_millis();
    let frame_motion = target / support::usize_as_f64(usize::try_from(frames).unwrap());
    let mut samples = vec![scroll_y(&mut app)];
    for _ in 0..frames + 2 {
        app.pump_for(waterui_testing::VIRTUAL_FRAME);
        samples.push(scroll_y(&mut app));
    }

    for pair in samples.windows(2) {
        let step = pair[1] - pair[0];
        assert!(
            (-0.5..=frame_motion + 0.5).contains(&step),
            "each frame must move the run by at most one frame of motion ({frame_motion}pt): \
             {} → {} in samples {samples:?}",
            pair[0],
            pair[1],
        );
    }
    let in_flight = samples
        .iter()
        .filter(|&&offset| offset > 0.5 && offset < target - 0.5)
        .count();
    assert!(
        in_flight + 2 >= usize::try_from(frames).unwrap(),
        "a {frames}-frame run must show its intermediate offsets frame by frame: {samples:?}"
    );
    let landed = *samples.last().expect("sampled the run");
    assert!(
        (landed - target).abs() < 1.0,
        "the run must land on row {SCROLL_TARGET} at {target}: {samples:?}"
    );
}

/// A membership change mid-flight must not kill an animated scroll: a chat
/// that calls `animate_to(last)` while messages keep arriving keeps its run
/// live — the membership re-anchor applies under it as a pure translation
/// (appending below leaves the anchor's coordinate unchanged) and the
/// request re-issues its target under the new membership, still landing on
/// its row (water-rs/waterui#1901).
#[test]
fn animate_to_survives_rows_inserted_during_the_flight_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..30).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.animate_to(29, waterui::animation::Animation::default());
    app.pump_for(std::time::Duration::from_millis(100));

    // Messages arriving while the scroll is in flight: the membership
    // re-anchor is a pure translation under the run — the appended rows
    // leave the anchor's coordinate unchanged — and the run must not read
    // the membership change as the user scrolling.
    let _ = items.replace((0..34).map(SelfId::new).collect());
    app.settle();

    let scroll_y = scroll_y(&mut app);
    // The request targets row 29 at 29·ROW_HEIGHT, but the appended rows
    // push it past the scrollable end: the run lands on the clamp — 34 rows
    // minus the 320pt viewport — with row 29 (and the true last row) in view.
    let expected = 34.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "the animated request must land on the clamped end despite the insertion: expected scroll_y≈{expected}, got {scroll_y}"
    );
    app.query().label("row 29").assert_exists();
    app.query().label("row 33").assert_exists();
}

/// The interrupt race: user input ends the run between frames, and rows
/// changing in that window must still re-anchor the viewport — the anchor
/// applies unconditionally (its translation inside the rebind cannot disturb
/// whatever now owns the offset) before the dead request drops, so content
/// does not jump under the user's finger (water-rs/waterui#1901).
#[test]
fn animate_to_interrupted_then_rows_change_still_anchors_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.scroll_to(20);
    app.settle();
    // A run long enough to still be in flight when the user's delta lands
    // four virtual frames in.
    controller.animate_to(
        0,
        waterui::animation::Animation::linear(std::time::Duration::from_millis(240)),
    );
    app.pump_for(std::time::Duration::from_millis(48));
    let in_flight = scroll_y(&mut app);
    assert!(
        in_flight > 120.0 && in_flight < 20.0 * ROW_HEIGHT,
        "the run must be mid-flight heading for row 0 when it is interrupted: {in_flight}"
    );

    // The user's pixel delta (finger down pushes the offset back into the
    // content) kills the run between frames; the membership event lands one
    // frame later — the window in which the dead request still waits for
    // its outcome, and the anchor record already holds the user's offset.
    app.queue_scroll_at(160.0, 160.0, 0.0, -120.0, false);
    app.pump_for(std::time::Duration::from_millis(16));
    let _ = items.replace((100..103).chain(0..40).map(SelfId::new).collect());
    app.settle();

    // The user scrolled `in_flight` back by 120, then the three prepended
    // rows re-anchored the viewport on the same content (+3 rows). A drop
    // would leave the offset 3 rows short of the content the user saw.
    let expected = 3.0f64.mul_add(ROW_HEIGHT, in_flight + 120.0);
    let scroll_y = scroll_y(&mut app);
    assert!(
        (scroll_y - expected).abs() < 30.0,
        "the anchor must still apply on the frame the request dies: expected scroll_y≈{expected}, got {scroll_y}"
    );
}

/// A request for a row the collection does not have yet waits for contents —
/// it is not live, so a membership change meanwhile must still re-anchor the
/// viewport instead of being dropped on the waiting request
/// (water-rs/waterui#1901).
#[test]
fn out_of_range_pending_scroll_still_anchors_membership_changes_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..30).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.scroll_to(15);
    app.settle();
    // Row 60 does not exist in the 30-row collection: the request waits, and
    // the viewport is the user's — not the request's.
    controller.animate_to(60, waterui::animation::Animation::default());
    app.pump_for(std::time::Duration::from_millis(32));

    // Three rows land above the viewport; the anchor keeps the parked
    // content in place (+3 rows of offset).
    let _ = items.replace((100..103).chain(0..30).map(SelfId::new).collect());
    app.settle();

    let scroll_y = scroll_y(&mut app);
    let expected = 18.0f64 * ROW_HEIGHT;
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "a waiting (out-of-range) request must not swallow the membership anchor: expected scroll_y≈{expected}, got {scroll_y}"
    );
}

/// A row inside the last screenful has an `offset_of` past `max_y`: the run
/// lands on the clamp, the row is already in view, and the request consumes.
#[test]
fn animate_to_a_row_in_the_last_screenful_lands_on_the_clamp_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..10).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.animate_to(9, waterui::animation::Animation::default());
    app.settle();

    let scroll_y = scroll_y(&mut app);
    // Row 9 sits at 9·ROW_HEIGHT, but 10 rows of content against a 320pt
    // viewport clamps the offset to max_y — exactly where a jump lands.
    let expected = 10.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "a last-screenful target must land on the clamp: expected scroll_y≈{expected}, got {scroll_y}"
    );
    app.query().label("row 9").assert_exists();
}

/// Rows taller than the Fenwick estimate: a tween that is too short for the
/// flight lands on whatever clamp the partially-measured extents allow — the
/// request stays armed past the landing and settles the offset with jumps as
/// materialization re-measures the rows ahead, not a second full-duration run
/// (the motion is over; settling the final position is a correction).
#[test]
fn animate_to_lands_then_corrects_as_rows_remeasure_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..20).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_list({
        let controller = controller.clone();
        move || {
            List::for_each(items.clone(), |item| {
                ListItem::new(vstack((text(format!("row {}", *item)),)).size(300.0, 112.0))
            })
            .scroll_controller(&controller)
            .a11y_label("messages")
        }
    });
    // Row 15's measured top is 15 x 132 = 1980 while the estimate puts it at
    // 1296: the 48ms tween runs out about three frames in, landing on the
    // early clamp around 1256 — short of the row. The landed run then keeps
    // correcting the offset by jumps until row 15's measured range reaches
    // the viewport, which must happen within a few frames — a re-armed
    // full-duration run would still be easing somewhere below.
    controller.animate_to(
        15,
        waterui::animation::Animation::linear(std::time::Duration::from_millis(48)),
    );
    app.pump_for(std::time::Duration::from_millis(160));
    app.query().label("row 15").assert_exists();
    let settled = scroll_y(&mut app);
    assert!(
        settled > 1400.0 && settled <= 1980.5,
        "the request stops where row 15 becomes visible — below its measured \
         start at 1980 and at least a viewport back (got scroll_y {settled})"
    );
}

/// The semantic runtime has no pump to advance an animation, so an animated
/// request lands in place there, in row units like any other request.
#[test]
fn animate_to_a_row_lands_in_place_semantic() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    controller.animate_to(SCROLL_TARGET, waterui::animation::Animation::default());
    let mut app = ui().mount({
        let controller = controller;
        move || pending_scroll_list(items.clone(), controller.clone())
    });
    app.settle();
    let scroll_y = scroll_y(&mut app);
    assert!(
        (scroll_y - support::usize_as_f64(SCROLL_TARGET)).abs() < 0.5,
        "animated request should land on row {SCROLL_TARGET} on the semantic runtime: scroll_y={scroll_y}"
    );
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

/// A membership change mid-fling must not stop the fling: the anchor
/// shifts the coordinate system — it does not claim the offset — and the
/// rebind a row insert triggers must not stale the claim the fling's
/// gesture-start handle holds, so the gesture keeps owning the offset
/// through the insertion (water-rs/waterui#1901).
#[test]
fn a_fling_survives_rows_inserted_above_the_viewport_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);
    // A touch drag is what claims a scroll view on a real device; the host
    // reports its gesture constants through `touch_scroll_config`.
    app.set_touch_scroll_config(hydrolysis::TouchScrollConfig::android_default());

    controller.scroll_to(20);
    app.settle();

    // A fast downward drag: the release velocity earns a fling that keeps
    // easing the offset back toward the top on its own spline.
    app.queue_drag_from_to_with(
        160.0,
        80.0,
        160.0,
        320.0,
        DragOptions {
            steps: 5,
            frame_per_step: true,
            pointer: hydrolysis::PointerKind::Touch,
        },
    );
    app.pump_for(std::time::Duration::from_millis(32));
    let fling_start = scroll_y(&mut app);
    app.pump_for(std::time::Duration::from_millis(32));
    let flinging = scroll_y(&mut app);
    assert!(
        flinging < fling_start - 5.0,
        "the release must produce a live fling before the insertion: {fling_start} → {flinging}"
    );

    // Three rows land above the viewport mid-fling.
    let _ = items.replace((100..103).chain(0..40).map(SelfId::new).collect());
    app.pump_for(std::time::Duration::from_millis(16));
    let at_insert = scroll_y(&mut app);
    app.pump_for(std::time::Duration::from_millis(48));
    let after_insert = scroll_y(&mut app);
    assert!(
        after_insert < at_insert - 10.0,
        "the fling must keep moving the offset through the membership change: \
         {at_insert} → {after_insert}"
    );
}

/// The membership anchor is a pure translation of the recorded offset —
/// `new_anchor − recorded`, never `new_anchor − current` — applied while a
/// run is `Running`: the insertion frame shows the in-flight offset plus the
/// inserted height plus exactly that frame's run motion. An un-anchored
/// insertion would miss the height; translating the live offset would erase
/// the frame's motion. The run then still lands on its re-issued target
/// (water-rs/waterui#1901).
#[test]
fn animate_to_running_reanchors_rows_inserted_above_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);
    controller.scroll_to(20);
    app.settle();
    // Row 20 (1120) to the 40-row end (1920) in 320ms: on the virtual
    // clock the run arms on the first pumped frame and moves 40pt every
    // 16ms frame, so it is still in flight on the insertion frame.
    controller.animate_to(
        38,
        waterui::animation::Animation::linear(std::time::Duration::from_millis(320)),
    );
    app.pump_for(std::time::Duration::from_millis(48));
    let previous = frame_scroll_y(&app);
    app.pump_for(std::time::Duration::from_millis(16));
    let in_flight = frame_scroll_y(&app);
    let frame_motion = in_flight - previous;
    assert!(
        frame_motion > 1.0 && in_flight < 1920.0,
        "the run must be mid-flight heading for row 38 when the rows land: {previous} → {in_flight}"
    );

    let _ = items.replace((100..103).chain(0..40).map(SelfId::new).collect());
    app.pump_for(std::time::Duration::from_millis(16));
    let at_insert = frame_scroll_y(&app);
    let expected = 3.0f64.mul_add(ROW_HEIGHT, in_flight) + frame_motion;
    assert!(
        (at_insert - expected).abs() < 1.0,
        "the insertion frame must add the three prepended rows to the in-flight offset and keep the frame's motion: \
         expected scroll_y≈{expected}, got {at_insert} (in flight {in_flight}, one frame {frame_motion})"
    );

    app.settle();
    let landed = scroll_y(&mut app);
    assert!(
        (landed - 2088.0).abs() < 1.0,
        "the run must still land on its target's clamped offset (43 rows − viewport): {landed}"
    );
}

/// Deleting rows above the viewport while scrolled to the bottom: the
/// membership translation and the shrunk extents' clamp are one operation,
/// so the offset lands on the new end — a clamp before the translation
/// would remove the deleted height twice (water-rs/waterui#1901).
#[test]
fn rows_deleted_above_the_bottom_land_on_the_new_end_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);
    controller.scroll_to(39);
    app.settle();
    let bottom = 40.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        (scroll_y(&mut app) - bottom).abs() < 1.0,
        "the list must start at its end"
    );

    let _ = items.replace((3..40).map(SelfId::new).collect());
    app.settle();
    let new_end = 37.0f64.mul_add(ROW_HEIGHT, -320.0);
    let landed = scroll_y(&mut app);
    assert!(
        (landed - new_end).abs() < 1.0,
        "deleting rows above the viewport at the bottom must land on the new end {new_end}: got {landed}"
    );
}

/// The same deletion while a run toward the end is in flight past the new
/// end: the run's offset, origin and target translate together before the
/// clamp, so the insertion frame shows the in-flight offset minus the
/// deleted height plus one frame of the run (water-rs/waterui#1901).
#[test]
fn rows_deleted_above_a_run_past_the_new_end_translate_once_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);
    controller.scroll_to(20);
    app.settle();
    controller.animate_to(
        36,
        waterui::animation::Animation::linear(std::time::Duration::from_millis(320)),
    );
    // The run arms on the first pumped frame and moves 40pt a frame.
    app.pump_for(std::time::Duration::from_millis(272));
    let previous = frame_scroll_y(&app);
    app.pump_for(std::time::Duration::from_millis(16));
    let in_flight = frame_scroll_y(&app);
    let new_end = 37.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        in_flight > new_end && in_flight < 40.0f64.mul_add(ROW_HEIGHT, -320.0),
        "the run must be in flight past the end the deletion leaves: {in_flight}"
    );

    let _ = items.replace((3..40).map(SelfId::new).collect());
    app.pump_for(std::time::Duration::from_millis(16));
    let at_delete = frame_scroll_y(&app);
    let expected = 3.0f64.mul_add(-ROW_HEIGHT, in_flight) + (in_flight - previous);
    assert!(
        (at_delete - expected).abs() < 1.0,
        "the deletion frame must translate the run once: expected scroll_y≈{expected}, got {at_delete} (in flight {in_flight})"
    );
    app.settle();
    let landed = scroll_y(&mut app);
    assert!(
        (landed - new_end).abs() < 1.0,
        "the run must land on the new end {new_end}: {landed}"
    );
}

/// A collection update between the frame's two `prepare_rows` passes must
/// add only its own rows. The first update prepends three rows and inserts
/// row 300 inside the viewport; the semantic pass resolves the anchor, then
/// materializes row 300, whose builder prepends three more — so the render
/// pass resolves the anchor again. The anchor is written back in the new
/// coordinates on resolution, so the first three rows are not translated a
/// second time (water-rs/waterui#1901).
///
/// The scenario depends on row 300 being built during the semantic pass —
/// `render_list_node` runs `list_accessibility` before `render_list_parts`.
/// The test asserts that the update's frame published the first update only
/// with the prepend already done, so a pass-order change fails here instead
/// of passing vacuously.
#[test]
fn an_update_between_the_frames_passes_translates_once_offscreen() {
    let items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let prepended = Rc::new(Cell::new(false));
    let mut app = mount_list({
        let items = items.clone();
        let controller = controller.clone();
        let prepended = Rc::clone(&prepended);
        move || {
            let history = items.clone();
            let prepended = Rc::clone(&prepended);
            List::for_each(items.clone(), move |item| {
                if *item == 300 && !prepended.replace(true) {
                    let _ = history.replace(
                        (200..203)
                            .chain(100..103)
                            .chain(0..22)
                            .chain([300])
                            .chain(22..40)
                            .map(SelfId::new)
                            .collect(),
                    );
                }
                ListItem::new(text(format!("row {}", *item)))
            })
            .scroll_controller(&controller)
            .a11y_label("messages")
        }
    });
    controller.scroll_to(20);
    app.settle();
    let before = scroll_y(&mut app);

    let _ = items.replace(
        (100..103)
            .chain(0..22)
            .chain([300])
            .chain(22..40)
            .map(SelfId::new)
            .collect(),
    );
    // One frame. The tree it publishes comes from the semantic pass, so it
    // reports the first update's three rows only while the builder has
    // already prepended: the second update landed between the two passes.
    app.pump_for(waterui_testing::VIRTUAL_FRAME);
    assert!(
        prepended.get(),
        "row 300's builder must have prepended its rows within the update's frame"
    );
    let first_update = 3.0f64.mul_add(ROW_HEIGHT, before);
    let semantic = frame_scroll_y(&app);
    assert!(
        (semantic - first_update).abs() < 1.0,
        "the frame's semantic pass must resolve the first update only, the prepend landing \
         after it: expected scroll_y≈{first_update}, got {semantic}"
    );
    app.settle();
    let expected = 6.0f64.mul_add(ROW_HEIGHT, before);
    let landed = scroll_y(&mut app);
    assert!(
        (landed - expected).abs() < 1.0,
        "six rows prepended above the anchor must translate the viewport by six rows: expected scroll_y≈{expected}, got {landed}"
    );
}
