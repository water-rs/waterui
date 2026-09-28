//! water-rs/hydrolysis#260 — an open overlay's painted bounds occlude every
//! hit region beneath them: a press that lands inside an open `.context_menu`
//! — the drawn presentation or the popup window it mounts — belongs to the
//! menu. The `.on_tap` a row underneath carries must never arm, let alone
//! fire.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use accesskit::Role;
use nami::collection::SignalCollection;
use waterui::component::list::{List, ListItem};
use waterui::component::text;
use waterui::prelude::ContextMenu;
use waterui::{AnyView, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::frame::Frame;
use waterui_layout::scroll::scroll;
use waterui_layout::stack::vstack;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const WINDOW: (u32, u32) = (400, 700);
const ROWS: usize = 5;
/// The row the `.context_menu` hangs off — second to last, so the menu's
/// items land squarely on the last row's tap region.
const MENU_ROW: usize = ROWS - 2;
const ROW_H: f32 = 48.0;
/// The last row's tap region is tall enough to cover the whole menu.
const TAP_ROW_H: f32 = 400.0;
/// The tail row's long-press minimum, in milliseconds.
const LONG_PRESS_MS: u32 = 50;

fn secondary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Secondary,
        },
    ]
}

fn primary_click(x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 2,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ]
}

/// A `scroll` over a `List`: filler rows, the menu row (a button carrying the
/// `.context_menu`, so its a11y bounds are the open target), then a last row
/// `tail` supplies — its gesture region covers the rest of the window.
fn chat_list_with_tail(
    tail: impl Fn() -> AnyView + 'static,
    menu: Rc<dyn Fn() -> ContextMenu>,
) -> AnyViewBuilder<AnyView> {
    let tail = Rc::new(tail);
    AnyViewBuilder::<AnyView>::new(move || {
        let rows = (0..ROWS).map(SelfId::new).collect::<Vec<_>>();
        AnyView::new(scroll(List::for_each(SignalCollection::new(rows), {
            let menu = menu.clone();
            let tail = tail.clone();
            move |row| {
                let index = row.into_inner();
                let content = if index == MENU_ROW {
                    AnyView::new(
                        Frame::new(button("Menu row").action(|| {}).context_menu(menu()))
                            .height(ROW_H)
                            .max_width(f32::INFINITY),
                    )
                } else if index == ROWS - 1 {
                    tail()
                } else {
                    AnyView::new(vstack((text(format!("Row {index}")),)).size(f32::INFINITY, ROW_H))
                };
                ListItem::new(content)
            }
        })))
    })
}

/// The last row carries an `.on_tap` region covering the rest of the window.
fn chat_list(taps: Rc<RefCell<u32>>, menu: Rc<dyn Fn() -> ContextMenu>) -> AnyViewBuilder<AnyView> {
    chat_list_with_tail(
        move || {
            let taps = taps.clone();
            AnyView::new(
                vstack((text("Tail row"),))
                    .size(f32::INFINITY, TAP_ROW_H)
                    .on_tap(move || *taps.borrow_mut() += 1),
            )
        },
        menu,
    )
}

/// Same list, with an `.on_long_press_gesture` on the last row instead —
/// occlusion must hold for every gesture kind the engine could arm, not
/// only taps.
fn chat_list_with_long_press(
    long_presses: Rc<RefCell<u32>>,
    menu: Rc<dyn Fn() -> ContextMenu>,
) -> AnyViewBuilder<AnyView> {
    chat_list_with_tail(
        move || {
            let long_presses = long_presses.clone();
            AnyView::new(
                vstack((text("Tail row"),))
                    .size(f32::INFINITY, TAP_ROW_H)
                    .on_long_press_gesture(LONG_PRESS_MS, move || {
                        *long_presses.borrow_mut() += 1;
                    }),
            )
        },
        menu,
    )
}

fn runtime(builder: AnyViewBuilder<AnyView>) -> HeadlessRuntime {
    HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    )
}

fn pump_until_settled(runtime: &mut HeadlessRuntime) {
    for _ in 0..64 {
        let _ = runtime.pump_at(false, Instant::now());
        if runtime.is_settled() {
            break;
        }
    }
}

/// The a11y bounds `label`'s button was last seen with.
fn bounds_of(runtime: &mut HeadlessRuntime, label: &str) -> accesskit::Rect {
    let mut found = None;
    for _ in 0..64 {
        if let Some(update) = runtime.pump_at(false, Instant::now()).tree_update
            && let Some((_, node)) = find_by_label(&update, Role::Button, label)
        {
            found = node.bounds();
        }
        if runtime.is_settled() {
            break;
        }
    }
    found.unwrap_or_else(|| panic!("{label} must emit a button with bounds"))
}

/// A menu item landing on a tap row's hit region must run its own command
/// and leave the row untouched — the drawn presentation's panel paints over
/// the row, so the press belongs to the menu.
#[test]
fn drawn_menu_item_press_does_not_fall_through_to_the_row_tap() {
    let command = Rc::new(Cell::new(false));
    let taps = Rc::new(RefCell::new(0_u32));
    let fired = command.clone();
    let menu = Rc::new(move || {
        let menu_command = fired.clone();
        ContextMenu::new(vec!["Star".action(move || menu_command.set(true))])
            .accessory(Frame::new(button("React").action(|| {})))
    });
    let mut runtime = runtime(chat_list(taps.clone(), menu));
    pump_until_settled(&mut runtime);

    let row = bounds_of(&mut runtime, "Menu row");
    let (row_x, row_y) = (
        ((row.x0 + row.x1) / 2.0) as f32,
        ((row.y0 + row.y1) / 2.0) as f32,
    );
    for event in secondary_click(row_x, row_y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    let (_, accessory_frame) = runtime
        .context_menu_presentation_frames()
        .expect("the drawn presentation mounts");
    assert!(
        accessory_frame.is_some(),
        "an accessory forces the drawn presentation"
    );
    let item = runtime
        .context_menu_row_frames()
        .first()
        .copied()
        .expect("the drawn menu emits a row per item");
    assert!(
        item.y0 >= row.y1,
        "the menu item must sit over the tap row beneath the menu row, got {item:?}"
    );

    for event in primary_click(item.center().x as f32, item.center().y as f32) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);

    assert!(command.get(), "the item's command must run");
    assert_eq!(
        *taps.borrow(),
        0,
        "the press must not reach the tap region underneath the menu"
    );
    assert!(
        runtime.context_menu_presentation_frames().is_none(),
        "an item choice closes the menu"
    );
}

/// The popup-window path: the menu mounts as its own window over the list,
/// and windowing-system routing delivers the press to it — the row's tap
/// region underneath is never consulted.
#[test]
fn popup_window_item_press_does_not_fall_through_to_the_row_tap() {
    let command = Rc::new(Cell::new(false));
    let taps = Rc::new(RefCell::new(0_u32));
    let fired = command.clone();
    let menu = Rc::new(move || {
        let menu_command = fired.clone();
        ContextMenu::new(vec!["Star".action(move || menu_command.set(true))])
    });
    let mut runtime = runtime(chat_list(taps.clone(), menu));
    pump_until_settled(&mut runtime);

    // Open at the menu row's bottom edge so the popup's first item lands on
    // the tap row's region beneath it.
    let row = bounds_of(&mut runtime, "Menu row");
    let (press_x, press_y) = (((row.x0 + row.x1) / 2.0) as f32, (row.y1 - 4.0) as f32);
    for event in secondary_click(press_x, press_y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    let frame = *runtime
        .popup_frames()
        .first()
        .expect("a plain secondary click mounts a popup window");

    // The item's a11y bounds are in the popup's local space; the press
    // address is its absolute position inside the window the runner shows.
    let mut item = None;
    for _ in 0..64 {
        if let Some(update) = runtime.pump_at(false, Instant::now()).tree_update
            && let Some((_, node)) = find_by_label(&update, Role::Button, "Star")
            && let Some(bounds) = node.bounds()
        {
            item = Some(bounds);
        }
        if runtime.is_settled() {
            break;
        }
    }
    let item = item.expect("the popup emits the item's bounds");
    let (x, y) = (
        frame.x() + ((item.x0 + item.x1) / 2.0) as f32,
        frame.y() + ((item.y0 + item.y1) / 2.0) as f32,
    );
    assert!(
        y > row.y1 as f32,
        "the popup item must sit over the tap row's region, got y={y} vs row bottom {}",
        row.y1
    );

    for event in primary_click(x, y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);

    assert!(command.get(), "the item's command must run");
    assert_eq!(
        *taps.borrow(),
        0,
        "the press must not reach the tap region underneath the popup"
    );
    assert!(
        runtime.popup_frames().is_empty(),
        "an item choice closes the popup"
    );
}

/// The same occlusion, proven gesture-kind independent: a press held on a
/// menu item must not arm the `.on_long_press_gesture` on the row beneath —
/// the engine's candidates are filtered by the occluder before it chooses
/// what to arm, so there is no recognizer for the hold to feed.
#[test]
fn drawn_menu_item_press_and_hold_does_not_fall_through_to_the_row_long_press() {
    let command = Rc::new(Cell::new(false));
    let long_presses = Rc::new(RefCell::new(0_u32));
    let fired = command.clone();
    let menu = Rc::new(move || {
        let menu_command = fired.clone();
        ContextMenu::new(vec!["Star".action(move || menu_command.set(true))])
            .accessory(Frame::new(button("React").action(|| {})))
    });
    let mut runtime = runtime(chat_list_with_long_press(long_presses.clone(), menu));
    pump_until_settled(&mut runtime);

    let row = bounds_of(&mut runtime, "Menu row");
    let (row_x, row_y) = (
        ((row.x0 + row.x1) / 2.0) as f32,
        ((row.y0 + row.y1) / 2.0) as f32,
    );
    for event in secondary_click(row_x, row_y) {
        runtime.push_input_event(event);
    }
    pump_until_settled(&mut runtime);
    let item = runtime
        .context_menu_row_frames()
        .first()
        .copied()
        .expect("the drawn menu emits a row per item");

    // Down on the item, then ticks past the long-press deadline while the
    // press is still held — the gesture engine's `handle_tick` is what a
    // long press fires on.
    let (x, y) = (item.center().x as f32, item.center().y as f32);
    let start = Instant::now();
    runtime.push_input_event(InputEvent::PointerDown {
        id: 2,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(false, start);
    for step in 1..=4u64 {
        let _ = runtime.pump_at(
            false,
            start + Duration::from_millis(u64::from(LONG_PRESS_MS) * step),
        );
    }
    runtime.push_input_event(InputEvent::PointerUp {
        id: 2,
        kind: PointerKind::Mouse,
        x,
        y,
        button: PointerButton::Primary,
    });
    let _ = runtime.pump_at(
        false,
        start + Duration::from_millis(u64::from(LONG_PRESS_MS) * 5),
    );
    pump_until_settled(&mut runtime);

    assert!(command.get(), "the item's command must run");
    assert_eq!(
        *long_presses.borrow(),
        0,
        "the held press must not reach the long-press region underneath the menu"
    );
}
