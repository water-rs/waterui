//! A nested `on_tap` target inside a `ListItem` row must be reachable on the
//! rendered runtime: the innermost interactive target under the pointer wins,
//! and the row still takes the press — selection and focus — when the press
//! lands outside every nested target. Regression coverage for
//! water-rs/hydrolysis#175: the rendered hit test resolved the row's press
//! target before nested `on_tap` targets, so a tap on a reaction pill or media
//! slot inside a message row did nothing while the semantic runtime dispatched
//! the same tap correctly.
//!
//! Both dispatch paths are the backend's machinery — the semantic walk's
//! retained action and the rendered runtime's hit test — so the tests stay
//! here under water-rs/waterui#1673, mounted through `SemanticRuntime` and
//! `HeadlessRuntime` under `TestStyle`.

mod support;

use std::cell::Cell;
use std::rc::Rc;

use accesskit::Role;
use support::{mount_rendered, mount_semantic, node_id_with_role, nodes};
use waterui::accessibility::{AccessibilityChildren, AccessibilityRole};
use waterui::component::list::{List, ListItem};
use waterui::component::{hstack, spacer, text};
use waterui::id::SelfId;
use waterui::reactive::collection::List as ReactiveList;
use waterui::{Binding, Signal as _, ViewExt};
use waterui_core::Environment;

const ROW_COUNT: i32 = 3;

fn row_items() -> ReactiveList<SelfId<i32>> {
    ReactiveList::from((1..=ROW_COUNT).map(SelfId::new).collect::<Vec<_>>())
}

fn row_label(row: i32) -> String {
    format!("Row {row}")
}

fn reaction_label(row: i32) -> String {
    format!("React {row}")
}

fn thumb_label(row: i32) -> String {
    format!("Thumb {row}")
}

/// A message-row-shaped item from the #175 report: the row's own label on the
/// leading edge and a reaction pill carrying a nested `on_tap` on the
/// trailing edge.
fn message_row(row: i32, reactions: Rc<Cell<i32>>) -> ListItem {
    ListItem::new(hstack((
        text(row_label(row)),
        spacer(),
        text(reaction_label(row))
            .on_tap(move || reactions.set(row))
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants),
    )))
}

/// The semantic walk dispatches a tap on the nested target to the target
/// itself: the pill fires and the row is not selected.
#[test]
fn nested_tap_fires_on_semantic_mount_without_selecting() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let reactions = Rc::new(Cell::new(0));
    let reactions_for_view = Rc::clone(&reactions);
    let mut app = mount_semantic(
        Environment::new(),
        move || {
            let reactions = Rc::clone(&reactions_for_view);
            List::for_each(row_items(), move |item| {
                message_row(item.into_inner(), Rc::clone(&reactions))
            })
            .selection(&binding)
        },
        360,
        240,
    );
    app.settle();

    let pill = node_id_with_role(app.tree(), Role::Button, &reaction_label(2));
    app.click(pill);
    assert_eq!(reactions.get(), 2, "the nested on_tap must fire");
    assert_eq!(
        selection.snapshot(),
        None,
        "a tap on the nested target must not select the row"
    );

    let row = nodes(app.tree())
        .find(|(_, node)| {
            node.role() == Role::ListItem
                && node
                    .label()
                    .is_some_and(|label| label.contains(&row_label(2)))
        })
        .map(|(id, _)| id)
        .expect("row 2 emits a list-item node");
    app.click(row);
    assert_eq!(
        selection.snapshot(),
        Some(2),
        "a tap on the row body selects the row"
    );
}

/// The rendered runtime sends the same pointer sequence through the hit test:
/// a press inside the pill's bounds fires the pill's `on_tap` and nothing
/// else; a press inside the row but outside the pill selects the row.
#[test]
fn nested_tap_fires_on_offscreen_mount_without_selecting() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let reactions = Rc::new(Cell::new(0));
    let reactions_for_view = Rc::clone(&reactions);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            let reactions = Rc::clone(&reactions_for_view);
            List::for_each(row_items(), move |item| {
                message_row(item.into_inner(), Rc::clone(&reactions))
            })
            .selection(&binding)
        },
        360,
        240,
    );
    app.settle();

    let pill = node_id_with_role(app.tree(), Role::Button, &reaction_label(2));
    app.tap_node(pill, 0.5, 0.5);
    assert_eq!(reactions.get(), 2, "the nested on_tap must fire");
    assert_eq!(
        selection.snapshot(),
        None,
        "a tap on the nested target must not select the row"
    );

    let label = node_id_with_role(app.tree(), Role::Label, &row_label(2));
    app.tap_node(label, 0.5, 0.5);
    assert_eq!(
        selection.snapshot(),
        Some(2),
        "a tap on the row body selects the row"
    );
}

/// A nested target centred in the row: its bounds cover the row's centre, so
/// a geometric rule that concedes centre-covering regions to the press loses
/// it. The ancestry rule claims it anyway — the thumb is registered by a
/// descendant of the row's own view.
fn centered_thumb_row(row: i32, reactions: Rc<Cell<i32>>) -> ListItem {
    ListItem::new(hstack((
        spacer(),
        text(thumb_label(row))
            .on_tap(move || reactions.set(row))
            .a11y_role(AccessibilityRole::Button)
            .a11y_children(AccessibilityChildren::ExcludeDescendants),
        spacer(),
    )))
}

/// The centred thumb claims the press on the semantic walk: its tap fires and
/// the row is not selected.
#[test]
fn centered_nested_tap_claims_press_on_semantic_mount() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let reactions = Rc::new(Cell::new(0));
    let reactions_for_view = Rc::clone(&reactions);
    let mut app = mount_semantic(
        Environment::new(),
        move || {
            let reactions = Rc::clone(&reactions_for_view);
            List::for_each(row_items(), move |item| {
                centered_thumb_row(item.into_inner(), Rc::clone(&reactions))
            })
            .selection(&binding)
        },
        360,
        240,
    );
    app.settle();

    let thumb = node_id_with_role(app.tree(), Role::Button, &thumb_label(2));
    app.click(thumb);
    assert_eq!(reactions.get(), 2, "the nested on_tap must fire");
    assert_eq!(
        selection.snapshot(),
        None,
        "a tap on the nested target must not select the row"
    );
}

/// On the rendered runtime a tap dead-centre on the row still reaches the
/// centred thumb — covering the press centre no longer concedes the press —
/// while a tap near the row's leading edge, outside the thumb, selects.
#[test]
fn centered_nested_tap_claims_press_on_offscreen_mount() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let reactions = Rc::new(Cell::new(0));
    let reactions_for_view = Rc::clone(&reactions);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            let reactions = Rc::clone(&reactions_for_view);
            List::for_each(row_items(), move |item| {
                centered_thumb_row(item.into_inner(), Rc::clone(&reactions))
            })
            .selection(&binding)
        },
        360,
        240,
    );
    app.settle();

    let thumb = node_id_with_role(app.tree(), Role::Button, &thumb_label(2));
    app.tap_node(thumb, 0.5, 0.5);
    assert_eq!(reactions.get(), 2, "the nested on_tap must fire");
    assert_eq!(
        selection.snapshot(),
        None,
        "a tap on the nested target must not select the row"
    );

    let row = nodes(app.tree())
        .filter(|(_, node)| node.role() == Role::ListItem)
        .map(|(id, _)| id)
        .nth(1)
        .expect("row 2 emits a list-item node");
    app.tap_node(row, 0.03, 0.5);
    assert_eq!(
        selection.snapshot(),
        Some(2),
        "a tap on the row body outside the thumb selects the row"
    );
}

/// A tap handler attached to the row's own view — the content sub-view's
/// root — is not a descendant of it, so it never claims the press: the row
/// selects AND the handler fires — the coexistence a nested `on_tap` no longer
/// needs geometry to distinguish.
fn tapped_row(row: i32, taps: Rc<Cell<i32>>) -> ListItem {
    ListItem::new(hstack((text(row_label(row)), spacer())).on_tap(move || taps.set(row)))
}

/// The semantic walk's row Click resolves the row activation: the gesture
/// observer on the content root carries no semantic node of its own to click,
/// so this runtime asserts the press half — selection — and the rendered
/// runtime covers the tap firing alongside it.
#[test]
fn row_level_on_tap_coexists_with_press_on_semantic_mount() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let taps = Rc::new(Cell::new(0));
    let taps_for_view = Rc::clone(&taps);
    let mut app = mount_semantic(
        Environment::new(),
        move || {
            let taps = Rc::clone(&taps_for_view);
            List::for_each(row_items(), move |item| {
                tapped_row(item.into_inner(), Rc::clone(&taps))
            })
            .selection(&binding)
        },
        360,
        240,
    );
    app.settle();

    let row = nodes(app.tree())
        .find(|(_, node)| {
            node.role() == Role::ListItem && node.label() == Some(row_label(2).as_str())
        })
        .map(|(id, _)| id)
        .expect("row 2 emits a list-item node");
    app.click(row);
    assert_eq!(
        selection.snapshot(),
        Some(2),
        "a tap on the row body selects the row"
    );
}

#[test]
fn row_level_on_tap_coexists_with_press_on_offscreen_mount() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let taps = Rc::new(Cell::new(0));
    let taps_for_view = Rc::clone(&taps);
    let mut app = mount_rendered(
        Environment::new(),
        move || {
            let taps = Rc::clone(&taps_for_view);
            List::for_each(row_items(), move |item| {
                tapped_row(item.into_inner(), Rc::clone(&taps))
            })
            .selection(&binding)
        },
        360,
        240,
    );
    app.settle();

    let row = nodes(app.tree())
        .find(|(_, node)| {
            node.role() == Role::ListItem && node.label() == Some(row_label(2).as_str())
        })
        .map(|(id, _)| id)
        .expect("row 2 emits a list-item node");
    app.tap_node(row, 0.5, 0.5);
    assert_eq!(
        selection.snapshot(),
        Some(2),
        "a tap on the row body selects the row"
    );
    assert_eq!(taps.get(), 2, "the row-level on_tap must also fire");
}
