//! `.dialog(...)`: presentation by binding, action roles, dismissal writing
//! the binding back, and the dialog's accessibility tree — semantic mount
//! plus an offscreen render for the modal layer (water-rs/waterui#1210).

use std::cell::Cell;
use std::rc::Rc;

use hydrolysis_m3::Material3;
use waterui::Binding;
use waterui::Signal as _;
use waterui::ViewExt as _;
use waterui::component::{text, vstack};
use waterui::dialog::{Dialog, DialogAction};
use waterui_controls::button;
use waterui_core::Environment;
use waterui_testing::{Role, ui};

/// The standard fixture: content that opens a two-action dialog on
/// `presented`; `flag` records the destructive action's handler.
fn fixture(presented: &Binding<bool>, flag: Rc<Cell<bool>>) -> impl waterui::View + use<> {
    let open = presented.clone();
    vstack((
        text("Content"),
        button("Open").action(move |_: Environment| open.set(true)),
    ))
    .dialog(
        Dialog::new(presented, "Delete photo?")
            .message("This cannot be undone.")
            .action(DialogAction::cancel("Cancel", || {}))
            .action(DialogAction::destructive(
                "Delete",
                move |_: Environment| {
                    flag.set(true);
                },
            )),
    )
}

#[test]
fn dialog_presents_by_binding_on_semantic_mount() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || fixture(&presented, flag.clone()));
    app.settle();
    app.query().role(Role::DIALOG).assert_not_exists();

    app.query()
        .role(Role::BUTTON)
        .label("Open")
        .single()
        .tap(&mut app);
    app.settle();

    app.query().role(Role::DIALOG).assert_exists();
    app.query().label("Delete photo?").assert_exists();
    app.query().label("This cannot be undone.").assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Cancel")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Delete")
        .assert_exists();
}

#[test]
fn presented_dialog_replaces_content_in_the_semantic_tree() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || fixture(&presented, flag.clone()));
    app.settle();
    app.query().label("Content").assert_exists();

    shown.set(true);
    app.settle();

    // The modal layer is the a11y surface while presented: the wrapped
    // content beneath is inert for accessibility (water-rs/waterui#1210).
    app.query().role(Role::DIALOG).assert_exists();
    app.query().label("Content").assert_not_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Open")
        .assert_not_exists();
}

#[test]
fn default_action_runs_handler_and_writes_binding_back() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let hit = flag.clone();
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || fixture(&presented, flag.clone()));
    app.settle();
    shown.set(true);
    app.settle();

    app.query()
        .role(Role::BUTTON)
        .label("Delete")
        .single()
        .tap(&mut app);
    app.settle();

    assert!(hit.get(), "the destructive handler never ran");
    assert!(
        !shown.snapshot(),
        "the action did not write is_presented back to false"
    );
    app.query().role(Role::DIALOG).assert_not_exists();
    app.query().label("Content").assert_exists();
}

#[test]
fn cancel_action_runs_its_handler_and_closes() {
    let presented = Binding::bool(false);
    let cancelled = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let c = cancelled.clone();
    let mut app = ui().viewport(400, 300).mount(move || {
        let c = c.clone();
        text("Content").dialog(
            Dialog::new(&presented, "Quit?")
                .action(DialogAction::cancel(
                    "Keep editing",
                    move |_: Environment| {
                        c.set(true);
                    },
                ))
                .action(DialogAction::new("Quit", || {})),
        )
    });
    app.settle();
    shown.set(true);
    app.settle();

    app.query()
        .role(Role::BUTTON)
        .label("Keep editing")
        .single()
        .tap(&mut app);
    app.settle();

    assert!(cancelled.get(), "the cancel handler never ran");
    assert!(!shown.snapshot());
    app.query().role(Role::DIALOG).assert_not_exists();
}

#[test]
fn escape_runs_the_cancel_path_and_closes() {
    let presented = Binding::bool(false);
    let cancelled = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let c = cancelled.clone();
    let mut app = ui().viewport(400, 300).mount(move || {
        let c = c.clone();
        text("Content").dialog(
            Dialog::new(&presented, "Discard draft?")
                .action(DialogAction::cancel("Cancel", move |_: Environment| {
                    c.set(true);
                }))
                .action(DialogAction::destructive("Discard", || {})),
        )
    });
    app.settle();
    shown.set(true);
    app.settle();

    app.press_named_key("Escape");
    app.settle();

    assert!(cancelled.get(), "Escape did not reach the cancel handler");
    assert!(!shown.snapshot());
}

#[test]
fn escape_without_a_cancel_action_keeps_the_dialog_up() {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let mut app = ui().viewport(400, 300).mount(move || {
        text("Content")
            .dialog(Dialog::new(&presented, "Heads up").action(DialogAction::new("OK", || {})))
    });
    app.settle();
    shown.set(true);
    app.settle();

    app.press_named_key("Escape");
    app.settle();

    assert!(
        shown.snapshot(),
        "Escape closed a dialog that declared no Cancel action"
    );
    app.query().role(Role::DIALOG).assert_exists();
}

#[test]
fn actionless_dialog_gains_one_ok_acknowledgement() {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || text("Content").dialog(Dialog::new(&presented, "Done")));
    app.settle();
    shown.set(true);
    app.settle();

    app.query()
        .role(Role::BUTTON)
        .label("OK")
        .single()
        .tap(&mut app);
    app.settle();
    assert!(!shown.snapshot());
}

#[test]
fn app_dismissal_closes_without_running_a_handler() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let hit = flag.clone();
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || fixture(&presented, flag.clone()));
    app.settle();
    shown.set(true);
    app.settle();
    app.query().role(Role::DIALOG).assert_exists();

    shown.set(false);
    app.settle();

    assert!(!hit.get(), "dismissing the binding ran an action handler");
    app.query().role(Role::DIALOG).assert_not_exists();
    app.query().label("Content").assert_exists();
}

#[test]
fn presented_dialog_is_in_the_offscreen_a11y_tree() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let mut app = ui()
        .viewport(400, 300)
        .theme(Material3::defaults())
        .mount_offscreen(move || fixture(&presented, flag.clone()));
    app.settle();
    app.query().role(Role::DIALOG).assert_not_exists();

    shown.set(true);
    app.settle();

    app.query().role(Role::DIALOG).assert_exists();
    app.query().label("Delete photo?").assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Delete")
        .assert_exists();
}

#[test]
#[should_panic(expected = "more than one Cancel")]
fn two_cancel_actions_fail_fast() {
    let presented = Binding::bool(true);
    let mut app = ui().viewport(400, 300).mount(move || {
        text("Content").dialog(
            Dialog::new(&presented, "Invalid")
                .action(DialogAction::cancel("Cancel", || {}))
                .action(DialogAction::cancel("Also cancel", || {})),
        )
    });
    app.settle();
}

#[test]
fn presented_dialog_renders_to_snapshot() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let mut app = ui()
        .viewport(800, 600)
        .theme(Material3::defaults())
        .mount_offscreen(move || fixture(&presented, flag.clone()));
    app.settle();
    shown.set(true);
    app.settle();

    let captured = app.capture_snapshot("dialog", "presented", "open");
    assert!(
        captured.path().exists(),
        "the offscreen render wrote no PNG at {}",
        captured.path().display()
    );
}
