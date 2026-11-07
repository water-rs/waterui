//! List scroll-internals regressions — the backend's own machinery:
//! `apply_scroll_request` pending targets, membership re-anchoring under
//! runs and flings, virtualization windows, and retained gesture orders.
//! These stay with the backend under water-rs/waterui#1673, mounted under
//! `TestStyle` (whose `one_line_row_height` is the 56pt figure the M3 mount
//! used) instead of `Material3`.
//!
//! <https://github.com/water-rs/hydrolysis/issues/168>: `apply_scroll_request`
//! asserted `index < row_count`, but the row count comes from signal-driven
//! contents — a `List` materializing mid-flush with a pending target above its
//! current row count panicked inside `list_accessibility`. The watergram
//! dogfood hits it on every chat open: the message list clears to 0 rows while
//! the previous contents' target is still pending. A scroll request names a
//! row that may not exist yet: it stays pending until the contents contain the
//! index, and a newer generation supersedes it.

mod support;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nami::Signal as _;
use nami::collection::List as ReactiveList;
use support::{
    count_role_label, mount_rendered, mount_semantic, node_id_contains, node_label_exists, nodes,
};
use waterui::component::list::{List, ListDelete, ListItem, ListMove};
use waterui::component::{hstack, spacer, text, vstack};
use waterui::id::SelfId;
use waterui::layout::scroll::ScrollController;
use waterui::{Binding, View, ViewExt};
use waterui_core::Environment;
use waterui_core::dynamic::watch;

/// A one-line list row under `TestStyle`: a scroll request landing on row
/// `N` reports a `scroll_y` of `N * ROW_HEIGHT` on the rendered runtime.
const ROW_HEIGHT: f64 = 56.0;
/// The pending target from the issue — a row the mounted contents do not have.
const SCROLL_TARGET: usize = 8;
/// The contents grow past the target after mount, so the pending request
/// becomes actionable.
const GROWN_ROW_COUNT: usize = 20;
/// One virtual frame.
const FRAME: std::time::Duration = std::time::Duration::from_millis(16);

/// Widens a `usize` count to `f64`, rounding to nearest.
#[expect(clippy::cast_precision_loss, reason = "test counts are far below 2^53")]
const fn usize_as_f64(v: usize) -> f64 {
    v as f64
}

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

/// Mounts `view` on the rendered runtime at the 320pt viewport the list
/// tests share and settles the first frames.
fn mount_list<V: View>(view: impl Fn() -> V + Clone + 'static) -> support::RenderedApp {
    let mut app = mount_rendered(Environment::new(), view, 320, 320);
    app.settle();
    app
}

/// Mounts [`pending_scroll_list`] over `items`, driven by `controller`.
fn mount_messages(
    items: &ReactiveList<SelfId<usize>>,
    controller: &ScrollController<usize>,
) -> support::RenderedApp {
    let items = items.clone();
    let controller = controller.clone();
    mount_list(move || pending_scroll_list(items.clone(), controller.clone()))
}

/// The `messages` list's reported vertical offset: points offscreen, row
/// units on the semantic runtime. Reading `app.tree()` never advances the
/// clock (water-rs/waterui#2243) — it is the frame-exact sample the
/// per-frame assertions need as well as the settled one.
fn scroll_y(tree: &accesskit::TreeUpdate) -> f64 {
    tree.nodes
        .iter()
        .find(|(_, node)| node.role() == accesskit::Role::List && node.label() == Some("messages"))
        .expect("the frame published the messages list")
        .1
        .scroll_y()
        .expect("the list reports a scroll offset")
}

/// The semantic runtime runs `list_accessibility` with no render context —
/// its scroll domain is row units, so a landed request reports the target
/// index as `scroll_y`.
#[test]
fn pending_scroll_target_above_row_count_waits_for_contents_semantic() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let controller = ScrollController::new(0);
    // The request lands while the contents are still empty: the dogfood's
    // cleared message list carried a pending target in exactly this shape.
    controller.scroll_to(SCROLL_TARGET);
    let mut app = mount_semantic(
        Environment::new(),
        {
            let items = items.clone();
            move || pending_scroll_list(items.clone(), controller.clone())
        },
        320,
        320,
    );
    // Growing the contents past the target makes the pending request
    // actionable; it must not have been dropped while unreachable.
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    app.settle();
    let scroll_y = scroll_y(app.tree());
    assert!(
        (scroll_y - usize_as_f64(SCROLL_TARGET)).abs() < 0.5,
        "pending scroll should land on row {SCROLL_TARGET} once the contents reach it: scroll_y={scroll_y}"
    );
}

/// The rendered runtime takes the same `apply_scroll_request` path through
/// `render_list`; its scroll domain is points, and the virtualized window
/// shows which rows are realized.
#[test]
fn pending_scroll_target_above_row_count_waits_for_contents_offscreen() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let controller = ScrollController::new(0);
    controller.scroll_to(SCROLL_TARGET);
    let mut app = mount_messages(&items, &controller);
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    app.settle();
    let scroll_y = scroll_y(app.tree());
    let expected = usize_as_f64(SCROLL_TARGET) * ROW_HEIGHT;
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "pending scroll should land on row {SCROLL_TARGET}: expected scroll_y≈{expected}, got {scroll_y}"
    );
    // The landed scroll moved the virtualized window: the target row is
    // realized, row 0 scrolled out and was evicted.
    assert!(
        node_label_exists(app.tree(), "row 8"),
        "the target row must be realized"
    );
    assert!(
        !node_label_exists(app.tree(), "row 0"),
        "row 0 must have scrolled out of the virtualized window"
    );
}

/// An `animate_to` request rides the request's curve to the row and lands
/// exactly where `scroll_to` would — the row's measured offset
/// (water-rs/waterui#1901). Mid-flight the reported `scroll_y` is strictly
/// between the two requests' rest and landing points.
#[test]
fn animate_to_a_row_glides_and_lands_where_the_jump_would_offscreen() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    // The jump lands on the row's offset immediately; it is the baseline the
    // animated request must converge to.
    controller.scroll_to(SCROLL_TARGET);
    app.settle();
    let jump_offset = scroll_y(app.tree());
    let expected = usize_as_f64(SCROLL_TARGET) * ROW_HEIGHT;
    assert!(
        (jump_offset - expected).abs() < 1.0,
        "jump should land on row {SCROLL_TARGET}'s offset ≈{expected}: {jump_offset}"
    );

    controller.scroll_to(0);
    app.settle();

    controller.animate_to(SCROLL_TARGET, waterui::animation::Animation::default());
    app.pump_for(std::time::Duration::from_millis(100));
    let midway = scroll_y(app.tree());
    assert!(
        midway > 0.0 && midway < jump_offset,
        "animated list scroll should be in flight mid-duration: scroll_y={midway}"
    );

    app.settle();
    // An animated list scroll must land on the same offset as the jump.
    approx::assert_relative_eq!(scroll_y(app.tree()), jump_offset);
    assert!(node_label_exists(app.tree(), "row 8"));
    assert!(!node_label_exists(app.tree(), "row 0"));
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.animate_to(
        SCROLL_TARGET,
        waterui::animation::Animation::linear(DURATION),
    );
    let target = usize_as_f64(SCROLL_TARGET) * ROW_HEIGHT;
    let frames = DURATION.as_millis() / FRAME.as_millis();
    let frame_motion = target / usize_as_f64(usize::try_from(frames).unwrap());
    let mut samples = vec![scroll_y(app.tree())];
    for _ in 0..frames + 2 {
        app.pump_for(FRAME);
        samples.push(scroll_y(app.tree()));
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
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

    let scroll_y = scroll_y(app.tree());
    // The request targets row 29 at 29·ROW_HEIGHT, but the appended rows
    // push it past the scrollable end: the run lands on the clamp — 34 rows
    // minus the 320pt viewport — with row 29 (and the true last row) in view.
    let expected = 34.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "the animated request must land on the clamped end despite the insertion: expected scroll_y≈{expected}, got {scroll_y}"
    );
    assert!(node_label_exists(app.tree(), "row 29"));
    assert!(node_label_exists(app.tree(), "row 33"));
}

/// The interrupt race: user input ends the run between frames, and rows
/// changing in that window must still re-anchor the viewport — the anchor
/// applies unconditionally (its translation inside the rebind cannot disturb
/// whatever now owns the offset) before the dead request drops, so content
/// does not jump under the user's finger (water-rs/waterui#1901).
#[test]
fn animate_to_interrupted_then_rows_change_still_anchors_offscreen() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
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
    let in_flight = scroll_y(app.tree());
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
    let scroll_y = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
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

    let scroll_y = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..10).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.animate_to(9, waterui::animation::Animation::default());
    app.settle();

    let scroll_y = scroll_y(app.tree());
    // Row 9 sits at 9·ROW_HEIGHT, but 10 rows of content against a 320pt
    // viewport clamps the offset to max_y — exactly where a jump lands.
    let expected = 10.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        (scroll_y - expected).abs() < 1.0,
        "a last-screenful target must land on the clamp: expected scroll_y≈{expected}, got {scroll_y}"
    );
    assert!(node_label_exists(app.tree(), "row 9"));
}

/// Rows taller than the Fenwick estimate: a tween that is too short for the
/// flight lands on whatever clamp the partially-measured extents allow — the
/// request stays armed past the landing and settles the offset with jumps as
/// materialization re-measures the rows ahead, not a second full-duration run
/// (the motion is over; settling the final position is a correction).
#[test]
fn animate_to_lands_then_corrects_as_rows_remeasure_offscreen() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
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
    assert!(node_label_exists(app.tree(), "row 15"));
    let settled = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    controller.animate_to(SCROLL_TARGET, waterui::animation::Animation::default());
    let mut app = mount_semantic(
        Environment::new(),
        move || pending_scroll_list(items.clone(), controller.clone()),
        320,
        320,
    );
    app.settle();
    let scroll_y = scroll_y(app.tree());
    assert!(
        (scroll_y - usize_as_f64(SCROLL_TARGET)).abs() < 0.5,
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
    let mut app = mount_rendered(
        Environment::new(),
        {
            let taps = Rc::clone(&taps);
            let highlight = highlight.clone();
            move || {
                let highlight = highlight.clone();
                let taps = Rc::clone(&taps);
                List::for_each([1, 2, 3], move |row: i32| {
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
        },
        360,
        240,
    );
    app.settle();

    let row2 = node_id_contains(app.tree(), "row 2");
    app.tap_node(row2, 0.5, 0.5);
    assert_eq!(taps.get(), 2, "baseline: row content on_tap must fire");

    // Rebuilding the row contents re-mints their tap targets while the rows'
    // swipe gestures stay retained; the taps must still outrank them.
    highlight.set(true);
    app.settle();

    taps.set(0);
    let row2 = node_id_contains(app.tree(), "row 2");
    app.tap_node(row2, 0.5, 0.5);
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
/// technology (and no `Click` on the node) could reach them. Each control now
/// emits a `Button` node at the drawn control bounds whose `Click` runs the
/// same handler its pointer target does.
#[test]
fn edit_controls_emit_accessibility_nodes_offscreen() {
    let deletes = Rc::new(Cell::new(usize::MAX));
    let moves = Rc::new(Cell::new((usize::MAX, usize::MAX)));
    let mut app = mount_rendered(
        Environment::new(),
        {
            let deletes = Rc::clone(&deletes);
            let moves = Rc::clone(&moves);
            move || {
                let deletes = Rc::clone(&deletes);
                let moves = Rc::clone(&moves);
                List::for_each([1, 2, 3], move |row: i32| {
                    ListItem::new(text(format!("row {row}")))
                })
                .editing(true)
                .on_delete(move |ListDelete(index)| deletes.set(index))
                .on_move(move |ListMove(reorder)| moves.set((reorder.from(), reorder.to())))
            }
        },
        360,
        240,
    );
    app.settle();

    // Three editing rows emit three delete controls; the reorder handle's up
    // half exists only where a row can move up, its down half where it can
    // move down — two each across three rows.
    assert_eq!(
        count_role_label(app.tree(), accesskit::Role::Button, "Delete"),
        3,
        "every editing row's delete control"
    );
    assert_eq!(
        count_role_label(app.tree(), accesskit::Role::Button, "Move up"),
        2,
        "every movable row's move-up control"
    );

    // `Click` performs the node's action — it must reach the same handler
    // the drawn control's pointer target runs.
    let delete_id = nodes(app.tree())
        .filter(|(_, node)| {
            node.role() == accesskit::Role::Button && node.label() == Some("Delete")
        })
        .map(|(id, _)| id)
        .nth(1)
        .expect("row 1's delete control");
    app.click(delete_id);
    assert_eq!(
        deletes.get(),
        1,
        "tapping row 1's delete control deletes it"
    );

    let move_down_id = nodes(app.tree())
        .filter(|(_, node)| {
            node.role() == accesskit::Role::Button && node.label() == Some("Move down")
        })
        .map(|(id, _)| id)
        .next()
        .expect("row 0's move-down control");
    app.click(move_down_id);
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
#[derive(Clone, PartialEq)]
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
    let mut app = mount_semantic(
        Environment::new(),
        {
            let items = items.clone();
            move || badge_list(nami::collection::SignalCollection::new(items.clone()))
        },
        320,
        320,
    );
    app.settle();
    assert!(node_label_exists(app.tree(), "badge-a"));

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

    assert!(node_label_exists(app.tree(), "badge-z"));
    assert!(!node_label_exists(app.tree(), "badge-a"));
    assert!(node_label_exists(app.tree(), "badge-b"));
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
    let mut app = mount_rendered(
        Environment::new(),
        {
            let items = items.clone();
            move || badge_list(nami::collection::SignalCollection::new(items.clone()))
        },
        320,
        320,
    );
    app.settle();
    let before = app.snapshot().rgba8().to_vec();

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
    let after = app.snapshot().rgba8().to_vec();
    assert!(
        after != before,
        "the updated row must repaint: a same-id content change must re-materialize \
         the retained row instead of replaying the stale subtree"
    );
    assert!(node_label_exists(app.tree(), "badge-z"));
}

/// A membership change mid-fling must not stop the fling: the anchor
/// shifts the coordinate system — it does not claim the offset — and the
/// rebind a row insert triggers must not stale the claim the fling's
/// gesture-start handle holds, so the gesture keeps owning the offset
/// through the insertion (water-rs/waterui#1901).
#[test]
fn a_fling_survives_rows_inserted_above_the_viewport_offscreen() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
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
        5,
        true,
        hydrolysis::PointerKind::Touch,
    );
    app.pump_for(std::time::Duration::from_millis(32));
    let fling_start = scroll_y(app.tree());
    app.pump_for(std::time::Duration::from_millis(32));
    let flinging = scroll_y(app.tree());
    assert!(
        flinging < fling_start - 5.0,
        "the release must produce a live fling before the insertion: {fling_start} → {flinging}"
    );

    // Three rows land above the viewport mid-fling.
    let _ = items.replace((100..103).chain(0..40).map(SelfId::new).collect());
    app.pump_for(std::time::Duration::from_millis(16));
    let at_insert = scroll_y(app.tree());
    app.pump_for(std::time::Duration::from_millis(48));
    let after_insert = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
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
    let previous = scroll_y(app.tree());
    app.pump_for(std::time::Duration::from_millis(16));
    let in_flight = scroll_y(app.tree());
    let frame_motion = in_flight - previous;
    assert!(
        frame_motion > 1.0 && in_flight < 1920.0,
        "the run must be mid-flight heading for row 38 when the rows land: {previous} → {in_flight}"
    );

    let _ = items.replace((100..103).chain(0..40).map(SelfId::new).collect());
    app.pump_for(std::time::Duration::from_millis(16));
    let at_insert = scroll_y(app.tree());
    let expected = 3.0f64.mul_add(ROW_HEIGHT, in_flight) + frame_motion;
    assert!(
        (at_insert - expected).abs() < 1.0,
        "the insertion frame must add the three prepended rows to the in-flight offset and keep the frame's motion: \
         expected scroll_y≈{expected}, got {at_insert} (in flight {in_flight}, one frame {frame_motion})"
    );

    app.settle();
    let landed = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..40).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);
    controller.scroll_to(39);
    app.settle();
    let bottom = 40.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        (scroll_y(app.tree()) - bottom).abs() < 1.0,
        "the list must start at its end"
    );

    let _ = items.replace((3..40).map(SelfId::new).collect());
    app.settle();
    let new_end = 37.0f64.mul_add(ROW_HEIGHT, -320.0);
    let landed = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
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
    let previous = scroll_y(app.tree());
    app.pump_for(std::time::Duration::from_millis(16));
    let in_flight = scroll_y(app.tree());
    let new_end = 37.0f64.mul_add(ROW_HEIGHT, -320.0);
    assert!(
        in_flight > new_end && in_flight < 40.0f64.mul_add(ROW_HEIGHT, -320.0),
        "the run must be in flight past the end the deletion leaves: {in_flight}"
    );

    let _ = items.replace((3..40).map(SelfId::new).collect());
    app.pump_for(std::time::Duration::from_millis(16));
    let at_delete = scroll_y(app.tree());
    let expected = 3.0f64.mul_add(-ROW_HEIGHT, in_flight) + (in_flight - previous);
    assert!(
        (at_delete - expected).abs() < 1.0,
        "the deletion frame must translate the run once: expected scroll_y≈{expected}, got {at_delete} (in flight {in_flight})"
    );
    app.settle();
    let landed = scroll_y(app.tree());
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
    let mut items = ReactiveList::<SelfId<usize>>::new();
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
                    let _ = history.clone().replace(
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
    let before = scroll_y(app.tree());

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
    app.pump_for(FRAME);
    assert!(
        prepended.get(),
        "row 300's builder must have prepended its rows within the update's frame"
    );
    let first_update = 3.0f64.mul_add(ROW_HEIGHT, before);
    let semantic = scroll_y(app.tree());
    assert!(
        (semantic - first_update).abs() < 1.0,
        "the frame's semantic pass must resolve the first update only, the prepend landing \
         after it: expected scroll_y≈{first_update}, got {semantic}"
    );
    app.settle();
    let expected = 6.0f64.mul_add(ROW_HEIGHT, before);
    let landed = scroll_y(app.tree());
    assert!(
        (landed - expected).abs() < 1.0,
        "six rows prepended above the anchor must translate the viewport by six rows: expected scroll_y≈{expected}, got {landed}"
    );
}

// Issue #1259: a list's controller reports the actual scroll position —
// first layout, applied requests — and never a pending request's target row.
#[test]
fn scroll_position_reports_actual_offset_not_pending_requests() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    let position = controller
        .position()
        .snapshot()
        .expect("the first painted layout reports a position");
    assert_eq!(position.offset().y, 0.0);
    assert!(position.at_start() && !position.at_end());
    assert!(position.content_size().height > position.viewport_size().height);

    // A queued row request must not present itself as the actual position.
    controller.scroll_to(GROWN_ROW_COUNT - 1);
    assert_eq!(
        controller.position().snapshot().unwrap().offset(),
        position.offset(),
        "a pending request must never pose as the actual position"
    );
    app.pump_for(FRAME);
    let applied = controller.position().snapshot().unwrap();
    assert!(
        applied.offset().y > 0.0,
        "the applied request reports the landed offset: {applied:?}"
    );
}

// Issue #1259: content growth with the offset held updates the reported
// geometry and edge flags — the tail the controller was on moves away.
#[test]
fn scroll_position_updates_edge_flags_when_content_grows() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    controller.scroll_to(GROWN_ROW_COUNT - 1);
    app.pump_for(FRAME);
    let bottom = controller.position().snapshot().unwrap();
    assert!(bottom.at_end(), "scrolled to the last row must be at_end");
    let extent = bottom.content_size().height;

    // Appending rows grows the extent; the offset holds while `at_end`
    // clears — geometry must report without a scroll happening.
    let _ = items.replace((0..GROWN_ROW_COUNT + 10).map(SelfId::new).collect());
    app.pump_for(FRAME);
    let after = controller.position().snapshot().unwrap();
    assert_eq!(after.offset(), bottom.offset());
    assert!(
        !after.at_end(),
        "growth past the tail must clear at_end, got {after:?}"
    );
    assert!(after.content_size().height > extent);
}

// Issue #1259: a position observer may mutate the list collection during
// the report write — the flush must not hold a borrow it would deadlock.
#[test]
fn scroll_position_watcher_may_mutate_the_list_collection() {
    let mut items = ReactiveList::<SelfId<usize>>::new();
    let _ = items.replace((0..GROWN_ROW_COUNT).map(SelfId::new).collect());
    let controller = ScrollController::new(0);
    let mut app = mount_messages(&items, &controller);

    let fired = Rc::new(Cell::new(false));
    // The report binding is the sink the handle writes — the same
    // post-write seam the backend reentry test proves; a `Computed`
    // watcher can sit behind lazy re-evaluation, which is not what this
    // borrow-safety regression is about.
    let _guard = controller.position_report().watch({
        let items = RefCell::new(items.clone());
        let fired = Rc::clone(&fired);
        move |_| {
            if !fired.replace(true) {
                let _ = items
                    .borrow_mut()
                    .replace((0..GROWN_ROW_COUNT + 5).map(SelfId::new).collect());
            }
        }
    });
    // A move publishes the position, whose write runs the watcher — and
    // the collection mutation it makes must not trip a held borrow.
    controller.scroll_to(3);
    app.settle();
    assert!(
        controller.position().snapshot().unwrap().offset().y > 0.0,
        "the applied scroll must have moved the reported offset"
    );
    assert!(
        fired.get(),
        "the position watcher ran without a borrow panic"
    );
}
