//! Accessibility-tree coverage for the framework-owned list selection
//! (water-rs/waterui#1226): pointer, keyboard and accessibility input all
//! write the same binding, and list rows carry the selected state.

use std::collections::BTreeSet;

use waterui::Binding;
use waterui::ViewExt as _;
use waterui::component::button;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::id::SelfId;
use waterui::reactive::collection::List as ReactiveList;
use waterui_testing::{Modifiers, Role, ui};

const ROW_COUNT: i32 = 5;

fn row_items() -> ReactiveList<SelfId<i32>> {
    ReactiveList::from((1..=ROW_COUNT).map(SelfId::new).collect::<Vec<_>>())
}

fn row_label(row: i32) -> String {
    format!("Row {row}")
}

/// Asserts that the accessibility tree's focus — the keyboard focus the
/// backend reports — is on `row`'s `ListItem` node.
macro_rules! assert_row_focused {
    ($app:expr, $row:expr) => {{
        let element = $app
            .query()
            .role(Role::LIST_ITEM)
            .label(row_label($row))
            .single();
        assert_eq!(
            $app.tree().focus(),
            element.id(),
            "expected keyboard focus on {:?}",
            row_label($row)
        );
    }};
}

/// Single selection: an accessibility click selects the row and writes the
/// binding, and a programmatic write shows the matching row as selected.
#[test]
fn single_selection_click_selects_and_programmatic_write_shows() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let mut app = ui().mount(move || {
        List::for_each(row_items(), |item| {
            ListItem::new(text(row_label(item.into_inner())))
        })
        .selection(&binding)
    });

    app.query().role(Role::LIST_ITEM).label(row_label(2)).tap();
    assert_eq!(selection.get(), Some(2));
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(2))
        .selected(true)
        .assert_exists();
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .selected(false)
        .assert_exists();

    selection.set(Some(4));
    app.settle();
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(4))
        .selected(true)
        .assert_exists();
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(2))
        .selected(false)
        .assert_exists();
}

/// Multiple selection: plain clicks reset the range, the toggle modifier
/// adds and removes single rows, and Shift extends from the anchor.
#[test]
fn multi_selection_toggle_modifier_and_shift_range() {
    let selection = Binding::container(BTreeSet::<i32>::new());
    let binding = selection.clone();
    let mut app = ui().mount(move || {
        List::for_each(row_items(), |item| {
            ListItem::new(text(row_label(item.into_inner())))
        })
        .multi_selection(&binding)
    });

    app.query().role(Role::LIST_ITEM).label(row_label(1)).tap();
    assert_eq!(selection.get(), BTreeSet::from([1]));

    let control = Modifiers {
        control: true,
        ..Modifiers::default()
    };
    app.press_named_key_with("ArrowDown", control);
    assert_eq!(selection.get(), BTreeSet::from([1, 2]));
    app.press_named_key_with("ArrowDown", control);
    assert_eq!(selection.get(), BTreeSet::from([1, 2, 3]));
    // Toggle removes the row the modifier lands on.
    app.press_named_key_with("ArrowUp", control);
    assert_eq!(selection.get(), BTreeSet::from([1, 3]));

    // A plain click resets the selection and re-anchors at that row.
    app.query().role(Role::LIST_ITEM).label(row_label(1)).tap();
    assert_eq!(selection.get(), BTreeSet::from([1]));
    let shift = Modifiers {
        shift: true,
        ..Modifiers::default()
    };
    app.press_named_key_with("ArrowDown", shift);
    assert_eq!(selection.get(), BTreeSet::from([1, 2]));
    app.press_named_key_with("ArrowDown", shift);
    assert_eq!(selection.get(), BTreeSet::from([1, 2, 3]));
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(3))
        .selected(true)
        .assert_exists();
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(4))
        .selected(false)
        .assert_exists();
}

/// Keyboard navigation with a selection mode: arrows move the selection and
/// the focus together, Home/End jump to the ends of the list.
#[test]
fn keyboard_navigation_moves_selection_and_focus() {
    let selection = Binding::container(Option::<i32>::None);
    let binding = selection.clone();
    let mut app = ui().mount(move || {
        List::for_each(row_items(), |item| {
            ListItem::new(text(row_label(item.into_inner())))
        })
        .selection(&binding)
    });

    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .focus();

    app.press_named_key("ArrowDown");
    assert_eq!(selection.get(), Some(2));
    assert_row_focused!(app, 2);

    app.press_named_key("End");
    assert_eq!(selection.get(), Some(ROW_COUNT));
    assert_row_focused!(app, ROW_COUNT);

    app.press_named_key("Home");
    assert_eq!(selection.get(), Some(1));
    assert_row_focused!(app, 1);

    // The first row consumes ArrowUp: selection and focus stay put.
    app.press_named_key("ArrowUp");
    assert_eq!(selection.get(), Some(1));
    assert_row_focused!(app, 1);
}

/// Keyboard navigation without a selection mode: arrows move only the focus
/// (a row's activation stays untouched) and Enter activates the row.
#[test]
fn keyboard_without_selection_moves_focus_only_and_enter_activates() {
    let taps = Binding::container(0_i32);
    let counter = taps.clone();
    let mut app = ui().mount(move || {
        let counter = counter.clone();
        List::for_each(row_items(), move |item| {
            let counter = counter.clone();
            ListItem::new(button(row_label(item.into_inner())).action(move || {
                counter.set(counter.get() + 1);
            }))
        })
    });

    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(1))
        .focus();
    app.press_named_key("ArrowDown");
    assert_row_focused!(app, 2);
    app.press_named_key("ArrowDown");
    assert_row_focused!(app, 3);
    assert_eq!(taps.get(), 0);

    app.press_named_key("Enter");
    assert_eq!(taps.get(), 1);
    assert_row_focused!(app, 3);
}

/// The real pointer path: a click on the row commits the selection while the
/// row's own tap handler still runs.
#[test]
fn pointer_click_selects_and_row_tap_handlers_still_run() {
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
                let counter = counter.clone();
                ListItem::new(text(row_label(item.into_inner())).on_tap(move || {
                    counter.set(counter.get() + 1);
                }))
            })
            .selection(&binding)
        });

    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(2))
        .tap_at(0.5, 0.5);
    assert_eq!(selection.get(), Some(2));
    assert_eq!(taps.get(), 1);
    app.query()
        .role(Role::LIST_ITEM)
        .label(row_label(2))
        .selected(true)
        .assert_exists();
}
