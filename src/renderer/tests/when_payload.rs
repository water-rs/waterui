//! water-rs/hydrolysis#251: a `when` payload materialized after mount must be
//! registered exactly as an initially mounted subtree — a `when` nested in
//! the payload materializes, and `button().action()` press slots take pointer
//! input. Both assertions drive input through `push_input_event` pointer
//! down/up — the hit-test path winit uses — never `tap_at`'s semantic
//! shortcut.

use std::time::Instant;

use accesskit::Role;
use nami::Binding;
use nami::Signal as _;
use waterui::ViewExt as _;
use waterui::reactive::collection::SignalCollection;
use waterui::widget::condition::when;
use waterui_controls::button::button;
use waterui_core::AnyView;
use waterui_core::handler::AnyViewBuilder;
use waterui_core::id::SelfId;
use waterui_layout::stack::{VStack, hstack, vstack};
use waterui_text::text;

use super::popup_windows::find_by_label;
use super::{MinimalTestTheme, test_environment};
use crate::HeadlessRuntime;
use crate::platform::{InputEvent, PointerButton, PointerKind};

const WINDOW: (u32, u32) = (320, 240);

/// A primary click at `(x, y)` — the two pointer events a winit click pumps
/// through the same hit-test targets these tests exercise.
fn pointer_click(runtime: &mut HeadlessRuntime, x: f32, y: f32) {
    for event in [
        InputEvent::PointerDown {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
        InputEvent::PointerUp {
            id: 1,
            kind: PointerKind::Mouse,
            x,
            y,
            button: PointerButton::Primary,
        },
    ] {
        runtime.push_input_event(event);
    }
}

/// Pumps until the runtime settles (with a cap), returning the last merged
/// tree update it published, if any.
fn pump_until_settled(runtime: &mut HeadlessRuntime) -> Option<accesskit::TreeUpdate> {
    let mut update = None;
    for _ in 0..64 {
        if let Some(tree) = runtime.pump_at(false, Instant::now()).tree_update {
            update = Some(tree);
        }
        if runtime.is_settled() {
            break;
        }
    }
    update
}

/// (a) A `when` nested inside another `when`'s payload materializes like a
/// mounted sibling — here the payload was inserted after mount, and the inner
/// flip is driven by a pointer tap on the payload's own row.
#[test]
fn nested_when_inside_a_when_payload_materializes() {
    let outer = Binding::container(false);
    let inner = Binding::container(false);
    let view_outer = outer.clone();
    let view_inner = inner.clone();
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let outer = view_outer.clone();
            let inner = view_inner.clone();
            AnyView::new(vstack((
                text("header"),
                when(outer, move || {
                    let tap = inner.clone();
                    vstack((
                        text("banner").padding().on_tap(move || tap.set(true)),
                        when(inner.clone(), || text("popup row")),
                    ))
                }),
            )))
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    assert!(pump_until_settled(&mut runtime).is_some());
    outer.set(true);
    let update = pump_until_settled(&mut runtime).expect("the payload republishes the tree");
    let (_, banner) =
        find_by_label(&update, Role::Label, "banner").expect("the outer payload materialized");
    let bounds = banner.bounds().expect("the row has bounds");

    // The inner flip is driven through the gesture path on the payload row.
    pointer_click(
        &mut runtime,
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    let update = pump_until_settled(&mut runtime).expect("the inner `when` republishes the tree");
    let (_, popup) = find_by_label(&update, Role::Label, "popup row")
        .expect("the nested `when` materialized inside the inserted payload");
    assert!(
        popup.bounds().is_some_and(|b| b.y1 > b.y0),
        "the nested `when` payload takes layout space"
    );
}

/// (b) A `button().action()` inside a `when` payload inserted after mount
/// takes pointer input: pointer down/up at its painted bounds fires the
/// action — the press slot `bind_interaction_target` registers.
#[test]
fn button_inside_a_when_payload_receives_pointer_input() {
    let show = Binding::container(false);
    let hits = Binding::container(0_i32);
    let probe = hits.clone();
    let view_show = show.clone();
    let view_hits = hits.clone();
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let show = view_show.clone();
            let hits = view_hits.clone();
            AnyView::new(vstack((
                text("header"),
                when(show, move || {
                    let hits = hits.clone();
                    button("Bump").action(move || hits.set(hits.snapshot() + 1))
                }),
            )))
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    assert!(pump_until_settled(&mut runtime).is_some());
    show.set(true);
    let update = pump_until_settled(&mut runtime).expect("the payload republishes the tree");
    let (_, bump) =
        find_by_label(&update, Role::Button, "Bump").expect("the inserted payload's button emits");

    let bounds = bump.bounds().expect("the button has bounds");
    pointer_click(
        &mut runtime,
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    let _ = pump_until_settled(&mut runtime);
    assert_eq!(
        probe.snapshot(),
        1,
        "the inserted payload's button fired on a pointer click"
    );
}

/// The mid-flush shape: a lazy row's builder flips `mid` while the stack
/// materializes it, so the `when` payload is inserted during the materialize
/// pass itself — the press slot still binds and the button still fires.
#[test]
fn button_inserted_mid_lazy_materialize_receives_pointer_input() {
    let items = Binding::container(vec![SelfId::new(0_u32)]);
    let mid = Binding::container(false);
    let hits = Binding::container(0_i32);
    let probe = hits.clone();
    let rows = items.clone();
    let mut runtime = HeadlessRuntime::new_for_tests(
        test_environment(),
        AnyViewBuilder::<AnyView>::new(move || {
            let rows = rows.clone();
            let mid = mid.clone();
            let hits = hits.clone();
            AnyView::new(vstack((
                VStack::for_each(SignalCollection::new(rows), {
                    let mid = mid.clone();
                    move |n: SelfId<u32>| {
                        let mid = mid.clone();
                        let n = n.into_inner();
                        // Flipping `mid` while the appended row materializes is the
                        // mid-flush insertion the issue reports.
                        if n == 1 {
                            mid.set(true);
                        }
                        AnyView::new(hstack((
                            text(waterui::Str::from(format!("row {n}"))).padding(),
                        )))
                    }
                }),
                when(mid.clone(), move || {
                    let hits = hits.clone();
                    button("MidBump").action(move || hits.set(hits.snapshot() + 1))
                }),
            )))
        }),
        WINDOW.0,
        WINDOW.1,
        MinimalTestTheme::default(),
    );

    assert!(pump_until_settled(&mut runtime).is_some());
    let mut next = items.snapshot();
    next.push(SelfId::new(next.len() as u32));
    items.set(next);
    let update = pump_until_settled(&mut runtime).expect("the insertion republishes the tree");
    let (_, bump) = find_by_label(&update, Role::Button, "MidBump")
        .expect("the mid-flush payload's button emits");

    let bounds = bump.bounds().expect("the button has bounds");
    pointer_click(
        &mut runtime,
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    let _ = pump_until_settled(&mut runtime);
    assert_eq!(
        probe.snapshot(),
        1,
        "the mid-flush payload's button fired on a pointer click"
    );
}
