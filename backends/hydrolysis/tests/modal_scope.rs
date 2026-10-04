//! Modal interaction scope registration: the scope belongs to the node
//! carrying the `ModalInteraction` metadata, so it must register on both the
//! rendered flush and the headless semantic walk whether or not the modal's
//! content binds an interaction target.
//!
//! Reproduces water-rs/hydrolysis#147.

use hydrolysis_m3::Material3;
use waterui::Binding;
use waterui::Signal as _;
use waterui::ViewExt as _;
use waterui::component::{text, vstack};
use waterui_backend_core::widget::ModalInteraction;
use waterui_controls::button;
use waterui_core::Environment;
use waterui_core::handler::SharedAction;
use waterui_testing::ui;

fn modal_scope(dismiss: Binding<bool>) -> ModalInteraction {
    ModalInteraction::new(
        true,
        SharedAction::new(move |_: Environment| dismiss.set(true)),
    )
}

#[test]
fn modal_escape_dismisses_on_semantic_mount() {
    let closed = Binding::bool(false);
    let esc = modal_scope(closed.clone());
    let mut app = ui()
        .viewport(300, 300)
        .mount(move || vstack((button("Inside").action(|| {}),)).with(esc.clone()));
    app.settle();
    app.press_named_key("Escape");
    app.settle();
    assert!(
        closed.snapshot(),
        "Escape did not reach the modal's dismiss action"
    );
}

#[test]
fn modal_escape_dismisses_on_offscreen_mount() {
    let closed = Binding::bool(false);
    let esc = modal_scope(closed.clone());
    let mut app = ui()
        .viewport(300, 300)
        .theme(Material3::defaults())
        .mount_offscreen(move || vstack((button("Inside").action(|| {}),)).with(esc.clone()));
    app.settle();
    app.press_named_key("Escape");
    app.settle();
    assert!(
        closed.snapshot(),
        "Escape did not reach the modal's dismiss action"
    );
}

#[test]
fn tap_only_modal_escape_dismisses_on_semantic_mount() {
    let closed = Binding::bool(false);
    let esc = modal_scope(closed.clone());
    let mut app = ui().viewport(300, 300).mount(move || {
        vstack((
            text("First row").on_tap(|| {}),
            text("Second row").on_tap(|| {}),
        ))
        .with(esc.clone())
    });
    app.settle();
    app.press_named_key("Escape");
    app.settle();
    assert!(
        closed.snapshot(),
        "Escape did not reach the modal's dismiss action"
    );
}

#[test]
fn tap_only_modal_escape_dismisses_on_offscreen_mount() {
    let closed = Binding::bool(false);
    let esc = modal_scope(closed.clone());
    let mut app = ui()
        .viewport(300, 300)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            vstack((
                text("First row").on_tap(|| {}),
                text("Second row").on_tap(|| {}),
            ))
            .with(esc.clone())
        });
    app.settle();
    app.press_named_key("Escape");
    app.settle();
    assert!(
        closed.snapshot(),
        "Escape did not reach the modal's dismiss action"
    );
}
