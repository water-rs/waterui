//! water-rs/waterui#1290 — non-primary pointer buttons route to gestures.
//!
//! A middle click fires a `.gesture(TapGesture::new().buttons(..MIDDLE..))`
//! handler and leaves press slots alone; a secondary click still resolves
//! the `.context_menu` under the point before any secondary-button gesture,
//! and `Other` buttons stay unrouted.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use accesskit::Role;
use nami::Signal as _;
use waterui::gesture::{PointerButtons, TapGesture};
use waterui::{AnyView, Binding, Color, ViewExt as _};
use waterui_controls::button::button;
use waterui_controls::menu::CommandExt as _;
use waterui_core::handler::AnyViewBuilder;
use waterui_layout::frame::Frame;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::platform::{InputEvent, PointerButton, PointerKind};
use crate::{HeadlessPumpResult, HeadlessRuntime};

const WINDOW: (u32, u32) = (320, 240);

fn click(button: PointerButton, x: f32, y: f32) -> [InputEvent; 2] {
    [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button,
        },
    ]
}

fn runtime(view: AnyView) -> HeadlessRuntime {
    let view = RefCell::new(Some(view));
    let builder = AnyViewBuilder::<AnyView>::new(move || {
        view.borrow_mut()
            .take()
            .expect("the test view is built once")
    });
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        builder,
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );
    for _ in 0..4 {
        let _ = runtime.pump(false);
    }
    runtime
}

fn click_and_pump(
    runtime: &mut HeadlessRuntime,
    button: PointerButton,
    x: f32,
    y: f32,
) -> HeadlessPumpResult {
    for event in click(button, x, y) {
        runtime.push_input_event(event);
    }
    runtime.pump_at(false, Instant::now())
}

/// A `.gesture(TapGesture::new().buttons(PointerButtons::MIDDLE), ..)` view
/// fires on a middle click while its sibling default tap does not.
#[test]
fn middle_click_fires_the_middle_button_tap_only() {
    let middle_fired = Rc::new(RefCell::new(0_u32));
    let default_fired = Rc::new(RefCell::new(0_u32));
    let middle_for_view = Rc::clone(&middle_fired);
    let default_for_view = Rc::clone(&default_fired);
    let mut runtime = runtime(AnyView::new(
        Frame::new(
            AnyView::new(Color::srgb_hex("#18181B"))
                .gesture(TapGesture::new(), move || {
                    *default_for_view.borrow_mut() += 1;
                })
                .gesture(
                    TapGesture::new().buttons(PointerButtons::MIDDLE),
                    move || {
                        *middle_for_view.borrow_mut() += 1;
                    },
                ),
        )
        .width(200.0)
        .height(200.0),
    ));

    click_and_pump(&mut runtime, PointerButton::Middle, 160.0, 120.0);

    assert_eq!(
        *middle_fired.borrow(),
        1,
        "the middle-buttons tap must fire on a middle click"
    );
    assert_eq!(
        *default_fired.borrow(),
        0,
        "the default primary-only tap must not fire on a middle click"
    );
}

/// With a `.context_menu` under the point, a secondary click opens the menu
/// and never reaches a secondary-button tap on the same view.
#[test]
fn secondary_click_prefers_the_context_menu_over_a_secondary_tap() {
    let tapped = Rc::new(RefCell::new(false));
    let tapped_for_view = Rc::clone(&tapped);
    let mut runtime = runtime(AnyView::new(
        Frame::new(AnyView::new(Color::srgb_hex("#18181B")).gesture(
            TapGesture::new().buttons(PointerButtons::SECONDARY),
            move || {
                *tapped_for_view.borrow_mut() = true;
            },
        ))
        .width(200.0)
        .height(200.0)
        .context_menu(vec!["Copy".action(|| {})]),
    ));

    let pump = click_and_pump(&mut runtime, PointerButton::Secondary, 160.0, 120.0);

    let update = pump
        .tree_update
        .expect("opening the menu must publish an accessibility tree");
    assert!(
        find_by_label(&update, Role::Button, "Copy").is_some(),
        "the context menu's items must merge into the tree"
    );
    assert!(
        !*tapped.borrow(),
        "the menu claims the secondary press — the tap must not fire"
    );
}

/// Press slots are primary-only: a middle click on a `Button` neither fires
/// its action nor commits on release, while a primary click still does.
#[test]
fn middle_click_does_not_click_a_button() {
    let fired = Binding::container(false);
    let fired_for_view = fired.clone();
    let mut runtime = runtime(AnyView::new(
        Frame::new(button("target").action(move || {
            fired_for_view.set(true);
        }))
        .width(200.0)
        .height(200.0),
    ));

    click_and_pump(&mut runtime, PointerButton::Middle, 160.0, 120.0);
    assert!(!fired.snapshot(), "a middle click must not click a Button");

    click_and_pump(&mut runtime, PointerButton::Primary, 160.0, 120.0);
    assert!(fired.snapshot(), "the primary click still clicks it");
}
