//! water-rs/hydrolysis#27 — a control clipped by an ancestor keeps its logical
//! bounds in the accessibility tree, and activating it dispatches its retained
//! action instead of a pointer press synthesized at the bounds' centre.
//!
//! Visibility is a projection resolved at the point of use:
//! [`HeadlessRuntime::accessibility_activation_point`] maps a point inside a
//! node's logical rectangle onto the fragment left by the node's clip chain
//! and the window bounds, so the callers that must produce a real point — a
//! testing `tap_at`, an automation `pointer tap` — land inside what the user
//! can see, and a control clipped away entirely fails the query loudly.

mod support {
    /// Narrows a finite `f64` coordinate to `f32`, rounding to nearest.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "test layout coordinates are finite and well inside f32 range"
    )]
    pub const fn f64_as_f32(v: f64) -> f32 {
        v as f32
    }
}

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::{
    Action as AccessibilityAction, ActionRequest as AccessibilityActionRequest, NodeId,
    Rect as AccessibilityRect, TreeId as AccessibilityTreeId,
};
use hydrolysis::{
    AccessibilityActivationPointError, HeadlessRuntime, InputEvent, PointerButton, PointerKind,
};
use hydrolysis_m3::Material3;
use waterui::component::list::{List, ListItem};
use waterui::component::{hstack, spacer, text};
use waterui::id::SelfId;
use waterui::reactive::collection::List as ReactiveList;
use waterui::{Binding, Signal as _, ViewExt as _};
use waterui_controls::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::{AnyView, Environment};
use waterui_layout::frame::Frame;
use waterui_layout::scroll::scroll_horizontal;
use waterui_testing::{Role, ui};

/// A 40 pt scroll leaves the first one-line row (56 pt under Material 3)
/// straddling the viewport's top edge with a 16 pt visible sliver — and that
/// sliver is padding only: the row's `on_tap` strip, centred inside the row
/// at content height, already lies fully above the viewport.
const SCROLL: f32 = 40.0;
/// A shallower scroll leaves the same row straddling with its `on_tap` strip
/// partially visible: the strip's own top is clipped away while its lower
/// span still crosses the viewport edge.
const PARTIAL_SCROLL: f32 = 28.0;
const ROW_COUNT: i32 = 10;

fn row_items() -> ReactiveList<SelfId<i32>> {
    ReactiveList::from((1..=ROW_COUNT).map(SelfId::new).collect::<Vec<_>>())
}

fn row_label(row: i32) -> String {
    format!("Row {row}")
}

/// Mounts a fixture's rendered twin on a headless runtime — the surface
/// `accessibility_activation_point` resolves against. The deterministic fonts
/// keep this mount's geometry identical to the `ui()` app's, so a point
/// resolved on one lands on the other.
fn headless(content: AnyViewBuilder<AnyView>, width: u32, height: u32) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        Environment::new(),
        content,
        width,
        height,
        Material3::defaults(),
    )
}

/// Pumps one frame per virtual display interval until the runtime reports
/// quiescence — the cadence `OffscreenApp`'s settle paces at, applied to the
/// bare runtime. The clock is virtual: `at` advances a frame per pump and
/// never backwards, because a finite animation, a gliding scroll or an armed
/// gesture deadline ends only once the frame instant passes its end — a loop
/// that keeps re-reading `Instant::now()` can burn its whole pump budget
/// inside that duration on a fast host.
fn settle(runtime: &mut HeadlessRuntime, at: &mut Instant) {
    for _ in 0..240 {
        *at += Duration::from_millis(16);
        runtime.pump_at(false, *at);
        if runtime.is_settled() {
            return;
        }
    }
    panic!("the headless runtime did not settle");
}

fn node_id(tree: &accesskit::TreeUpdate, label: &str) -> NodeId {
    tree.nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .map_or_else(
            || panic!("no accessibility node labelled {label}"),
            |(id, _)| *id,
        )
}

fn node_bounds(tree: &accesskit::TreeUpdate, label: &str) -> AccessibilityRect {
    tree.nodes
        .iter()
        .find(|(_, node)| node.label() == Some(label))
        .and_then(|(_, node)| node.bounds())
        .unwrap_or_else(|| panic!("no bounded accessibility node labelled {label}"))
}

fn semantic_click(runtime: &mut HeadlessRuntime, at: &mut Instant, node: NodeId) {
    runtime.perform_accessibility_action(AccessibilityActionRequest {
        action: AccessibilityAction::Click,
        target_tree: AccessibilityTreeId::ROOT,
        target_node: node,
        data: None,
    });
    settle(runtime, at);
}

/// A pointer tap through the real input path — the down/up pair
/// `waterui-testing`'s `tap_at` pushes.
fn pointer_tap(runtime: &mut HeadlessRuntime, at: &mut Instant, x: f64, y: f64) {
    for event in [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Touch,
            x: support::f64_as_f32(x),
            y: support::f64_as_f32(y),
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Touch,
            x: support::f64_as_f32(x),
            y: support::f64_as_f32(y),
            button: PointerButton::Primary,
        },
    ] {
        runtime.push_input_event(event);
    }
    settle(runtime, at);
}

/// "Reset" sits 170 pt into a 200 pt horizontal rail: the logical rect's
/// centre lands past the rail's edge, inside the clipped region. The reported
/// bounds stay the full logical rectangle; the activation query resolves a
/// point inside the visible fragment; the retained action fires by pointer
/// and by semantic `Click`; and a control clipped away entirely fails the
/// query instead of dead-clicking.
#[test]
fn a_button_clipped_by_a_scroll_rail_activates_through_the_clip() {
    let count = Rc::new(Cell::new(0));
    let count_for_view = Rc::clone(&count);
    let mut runtime = headless(
        AnyViewBuilder::new(move || {
            let count = Rc::clone(&count_for_view);
            AnyView::new(
                scroll_horizontal(hstack((
                    spacer().size(170.0, 40.0),
                    button("Reset").action(move || count.set(count.get() + 1)),
                    button("Ghost").action(|| {}),
                )))
                .size(200.0, 60.0)
                .a11y_label("rail"),
            )
        }),
        240,
        240,
    );
    let mut at = Instant::now();
    settle(&mut runtime, &mut at);
    let tree = runtime
        .accessibility_tree()
        .expect("a mounted window emits a tree");

    let rail = node_bounds(&tree, "rail");
    let reset = node_id(&tree, "Reset");
    let bounds = node_bounds(&tree, "Reset");

    // The reported bounds are the logical rectangle: they reach past the
    // rail's clip edge, where no pointer can hit. Visibility is the query's
    // concern, not the tree's.
    assert!(
        bounds.x1 > rail.x1 + 1.0,
        "the clipped button reports its full logical bounds: bounds={bounds:?} rail={rail:?}"
    );

    // A caller that must produce a real point resolves one through the clip
    // projection: it lands inside the visible fragment, so the tap hits.
    let point = runtime
        .accessibility_activation_point(reset, 0.5, 0.5)
        .expect("a partially visible button resolves a hittable point");
    assert!(
        point.x < rail.x1 && point.y < rail.y1 && point.x >= rail.x0 && point.y >= rail.y0,
        "the resolved point lands inside the clip: {point:?} rail={rail:?}"
    );
    pointer_tap(&mut runtime, &mut at, point.x, point.y);
    assert_eq!(count.get(), 1, "a tap at the resolved point must fire");

    // The semantic Click dispatches the retained action directly — no point
    // is synthesized, so clipping the centre can no longer dead-click it.
    semantic_click(&mut runtime, &mut at, reset);
    assert_eq!(count.get(), 2, "a semantic Click fires even while clipped");

    // "Ghost" lies past the rail entirely: the query fails loudly rather than
    // handing a caller an off-screen point.
    let ghost = node_id(&tree, "Ghost");
    assert!(
        runtime
            .accessibility_activation_point(ghost, 0.5, 0.5)
            .is_err(),
        "a fully clipped control fails the projection query"
    );
}

/// The same contract at the list row, in its hardest form: the straddling
/// row reports its logical bounds and its 16 pt sliver is real — but that
/// sliver is padding only. The `on_tap` strip the row's `Click` answers for
/// lies fully above the viewport, so the projection must fail loudly rather
/// than land a dead tap on the padding, while the semantic `Click` still
/// dispatches the retained `on_tap` — plus its selection write — because it
/// never needed a point at all.
#[test]
fn a_list_row_straddling_the_viewport_edge_activates_its_retained_action() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let reactions = Rc::new(Cell::new(0));
    let reactions_for_view = Rc::clone(&reactions);
    let list_view = move || {
        let reactions = Rc::clone(&reactions_for_view);
        List::for_each(row_items(), move |item| {
            let row = item.into_inner();
            let reactions = Rc::clone(&reactions);
            ListItem::new(
                hstack((text(row_label(row)), spacer()))
                    .on_tap(move || reactions.set(reactions.get() + 1)),
            )
        })
        .selection(&binding)
    };
    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(360, 240)
        .mount_offscreen(list_view.clone());
    app.settle();
    app.scroll_at(180.0, 120.0, 0.0, -SCROLL, false);
    app.pump_for(Duration::from_millis(500));

    let list = app.query().role(Role::LIST).single().bounds();
    let row = app
        .query()
        .role(Role::LIST_ITEM)
        .label_contains(row_label(1))
        .single()
        .bounds();
    assert!(
        row.y() < list.y() && row.y() + row.height() > list.y(),
        "the straddling row reports its full logical bounds: row={row:?} list={list:?}"
    );

    // The projection projects the interaction owner's region — the `on_tap`
    // strip — through its clip, not the row's logical rectangle: at this
    // scroll the strip is fully clipped, so the query fails loudly instead
    // of producing a point on the padding sliver that taps into nothing.
    let mut runtime = headless(AnyViewBuilder::new(list_view).erase(), 360, 240);
    let mut at = Instant::now();
    settle(&mut runtime, &mut at);
    runtime.push_input_event(InputEvent::Scroll {
        x: 180.0,
        y: 120.0,
        dx: 0.0,
        dy: -SCROLL,
        is_line_delta: false,
    });
    settle(&mut runtime, &mut at);
    let tree = runtime
        .accessibility_tree()
        .expect("a mounted window emits a tree");
    let straddled = node_id(&tree, &row_label(1));
    assert_eq!(
        runtime.accessibility_activation_point(straddled, 0.5, 0.5),
        Err(AccessibilityActivationPointError::EmptyFragment),
        "a fully clipped interaction region fails the projection loudly"
    );

    // The semantic Click still dispatches the retained activation — the row
    // stands in for the silenced gesture and never needed a point.
    app.query()
        .role(Role::LIST_ITEM)
        .label_contains(row_label(1))
        .tap();
    assert_eq!(
        reactions.get(),
        1,
        "the semantic Click dispatches the row's retained activation"
    );
    assert_eq!(selection.snapshot(), Some(1), "the row still selects");
}

/// The neighbouring case on the same fixture: a shallower scroll leaves the
/// straddling row's `on_tap` strip *partially* visible — its own top clipped
/// away, its lower span inside the viewport. The projection resolves a point
/// inside the surviving fragment, and a real tap there fires the gesture.
#[test]
fn a_list_row_partially_straddling_resolves_a_point_inside_the_strip() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection;
    let reactions = Rc::new(Cell::new(0));
    let reactions_for_view = Rc::clone(&reactions);
    let list_view = move || {
        let reactions = Rc::clone(&reactions_for_view);
        List::for_each(row_items(), move |item| {
            let row = item.into_inner();
            let reactions = Rc::clone(&reactions);
            ListItem::new(
                hstack((text(row_label(row)), spacer()))
                    .on_tap(move || reactions.set(reactions.get() + 1)),
            )
        })
        .selection(&binding)
    };
    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(360, 240)
        .mount_offscreen(list_view.clone());
    app.settle();
    app.scroll_at(180.0, 120.0, 0.0, -PARTIAL_SCROLL, false);
    app.pump_for(Duration::from_millis(500));

    let list = app.query().role(Role::LIST).single().bounds();
    let row = app
        .query()
        .role(Role::LIST_ITEM)
        .label_contains(row_label(1))
        .single()
        .bounds();
    assert!(
        row.y() < list.y() && row.y() + row.height() > list.y(),
        "the straddling row reports its full logical bounds: row={row:?} list={list:?}"
    );

    let mut runtime = headless(AnyViewBuilder::new(list_view).erase(), 360, 240);
    let mut at = Instant::now();
    settle(&mut runtime, &mut at);
    runtime.push_input_event(InputEvent::Scroll {
        x: 180.0,
        y: 120.0,
        dx: 0.0,
        dy: -PARTIAL_SCROLL,
        is_line_delta: false,
    });
    settle(&mut runtime, &mut at);
    let tree = runtime
        .accessibility_tree()
        .expect("a mounted window emits a tree");
    let straddled = node_id(&tree, &row_label(1));

    // The strip's top edge is gone: asking for it clamps onto the clip's own
    // edge, which is what "partially visible" means here — a fully visible
    // strip would hand its own top back untouched.
    let top = runtime
        .accessibility_activation_point(straddled, 0.5, 0.0)
        .expect("a partially visible strip resolves a hittable point");
    assert!(
        top.y <= f64::from(list.y()) + 1.0,
        "the strip's clipped top edge resolves at the clip, not inside the strip: {top:?} list={list:?}"
    );

    let point = runtime
        .accessibility_activation_point(straddled, 0.5, 0.5)
        .expect("a partially visible strip resolves a hittable point");
    assert!(
        point.y >= f64::from(list.y()),
        "the resolved point lands inside the viewport: {point:?} list={list:?}"
    );
    pointer_tap(&mut runtime, &mut at, point.x, point.y);
    assert_eq!(
        reactions.get(),
        1,
        "a tap at the resolved point on the visible strip fragment fires"
    );
}

/// A context menu opened by a secondary click mounts as a popup window whose
/// nodes carry the window-id stride. The projection demuxes the shifted id to
/// the popup's own core — its clip and window bounds — and translates the
/// resolved point into the merged coordinates `push_input_event` resolves
/// against, so a real tap at the returned point routes into the popup and
/// fires the item.
#[test]
fn a_context_menu_item_in_a_popup_resolves_a_hittable_point() {
    let fired = Rc::new(Cell::new(0));
    let fired_for_view = Rc::clone(&fired);
    let mut runtime = headless(
        AnyViewBuilder::new(move || {
            let fired = Rc::clone(&fired_for_view);
            AnyView::new(
                Frame::new(text("Anchor"))
                    .width(120.0)
                    .height(40.0)
                    .context_menu(vec!["Copy".action(move || fired.set(fired.get() + 1))]),
            )
        }),
        240,
        240,
    );
    let mut at = Instant::now();
    settle(&mut runtime, &mut at);

    // A secondary press on the anchor mounts the menu as a popup window.
    for event in [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x: 10.0,
            y: 10.0,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x: 10.0,
            y: 10.0,
            button: PointerButton::Secondary,
        },
    ] {
        runtime.push_input_event(event);
    }
    settle(&mut runtime, &mut at);

    let tree = runtime
        .accessibility_tree()
        .expect("a mounted window emits a tree");
    let copy = node_id(&tree, "Copy");
    assert!(
        copy.0 >= (1 << 32),
        "the popup item carries the window-id stride: {copy:?}"
    );

    // The resolved point sits in merged coordinates: the tap routes into the
    // popup window's frame and fires the item, not the main window behind it.
    let point = runtime
        .accessibility_activation_point(copy, 0.5, 0.5)
        .expect("a popup item resolves a hittable point");
    pointer_tap(&mut runtime, &mut at, point.x, point.y);
    assert_eq!(
        fired.get(),
        1,
        "a tap at the resolved point routes into the popup and fires"
    );
}
