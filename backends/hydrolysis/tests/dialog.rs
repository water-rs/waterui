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
use waterui_testing::{AccessKitRole, Role, RuntimeDriver, SemanticApp, ui};

/// A presented dialog publishes its card as the window's modal alert.
const ALERT: Role = Role::new(AccessKitRole::AlertDialog);

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
    app.query().role(ALERT).assert_not_exists();

    app.query()
        .role(Role::BUTTON)
        .label("Open")
        .single()
        .tap(&mut app);
    app.settle();

    app.query()
        .role(ALERT)
        .label("Delete photo?")
        .assert_exists();
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
    app.query().role(ALERT).assert_exists();
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
    app.query().role(ALERT).assert_not_exists();
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
    app.query().role(ALERT).assert_not_exists();
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
    app.query().role(ALERT).assert_exists();
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
    app.query().role(ALERT).assert_exists();

    shown.set(false);
    app.settle();

    assert!(!hit.get(), "dismissing the binding ran an action handler");
    app.query().role(ALERT).assert_not_exists();
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
    app.query().role(ALERT).assert_not_exists();

    shown.set(true);
    app.settle();

    app.query().role(ALERT).assert_exists();
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

/// A window with real content beneath the modal layer, so the capture shows
/// whether the scrim dims it or paints over it.
fn content_fixture(presented: &Binding<bool>) -> impl waterui::View + use<> {
    let open = presented.clone();
    vstack((
        text("Inbox").title(),
        text("Quarterly report draft — shared with the finance team"),
        text("Design review notes from Thursday"),
        text("Travel itinerary for the October offsite"),
        button("Open").action(move |_: Environment| open.set(true)),
    ))
    .dialog(
        Dialog::new(presented, "Delete 3 messages?")
            .message("Deleted messages move to the trash and are removed after 30 days.")
            .action(DialogAction::cancel("Cancel", || {}))
            .action(DialogAction::destructive("Delete", || {})),
    )
}

fn capture_presented(style: Material3, case: &str) {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let mut app = ui()
        .viewport(800, 600)
        .theme(style)
        .mount_offscreen(move || content_fixture(&presented));
    app.settle();
    shown.set(true);
    app.settle();

    let captured = app.capture_snapshot("dialog", case, "presented");
    assert!(
        captured.path().exists(),
        "the offscreen render wrote no PNG at {}",
        captured.path().display()
    );
}

#[test]
fn presented_dialog_renders_over_content_light() {
    capture_presented(Material3::defaults(), "light");
}

#[test]
fn presented_dialog_renders_over_content_dark() {
    capture_presented(Material3::dark(), "dark");
}

/// A dialog attached to one view of a larger window: the sibling content
/// lies outside the `.dialog` subtree but beneath the modal layer.
fn sibling_fixture(presented: &Binding<bool>) -> impl waterui::View + use<> {
    vstack((
        text("Sidebar"),
        button("Sibling action").action(|| {}),
        text("Editor").dialog(
            Dialog::new(presented, "Close editor?")
                .action(DialogAction::cancel("Stay", || {}))
                .action(DialogAction::new("Close", || {})),
        ),
    ))
}

#[test]
fn presented_dialog_makes_the_whole_window_inert_on_semantic_mount() {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || sibling_fixture(&presented));
    app.settle();
    app.query().label("Sidebar").assert_exists();

    shown.set(true);
    app.settle();

    app.query().role(ALERT).assert_exists();
    app.query().label("Sidebar").assert_not_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Sibling action")
        .assert_not_exists();
}

#[test]
fn presented_dialog_makes_the_whole_window_inert_on_offscreen_mount() {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let mut app = ui()
        .viewport(400, 300)
        .theme(Material3::defaults())
        .mount_offscreen(move || sibling_fixture(&presented));
    app.settle();
    app.query().label("Sidebar").assert_exists();

    shown.set(true);
    app.settle();

    app.query().role(ALERT).assert_exists();
    app.query().label("Sidebar").assert_not_exists();
    app.query().label("Editor").assert_not_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Sibling action")
        .assert_not_exists();
}

#[test]
fn return_runs_the_primary_action() {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let saved = Rc::new(Cell::new(false));
    let hit = saved.clone();
    let mut app = ui().viewport(400, 300).mount(move || {
        let saved = saved.clone();
        text("Content").dialog(
            Dialog::new(&presented, "Save changes?")
                .action(DialogAction::cancel("Cancel", || {}))
                .action(DialogAction::destructive("Don't Save", || {}))
                .action(DialogAction::new("Save", move |_: Environment| {
                    saved.set(true);
                })),
        )
    });
    app.settle();
    shown.set(true);
    app.settle();

    app.press_named_key("Enter");
    app.settle();

    assert!(hit.get(), "Return did not run the primary action");
    assert!(!shown.snapshot());
}

#[test]
fn scrim_tap_runs_the_cancel_path() {
    let presented = Binding::bool(false);
    let shown = presented.clone();
    let cancelled = Rc::new(Cell::new(false));
    let hit = cancelled.clone();
    let mut app = ui()
        .viewport(400, 300)
        .theme(Material3::defaults())
        .mount_offscreen(move || {
            let cancelled = cancelled.clone();
            text("Content").dialog(
                Dialog::new(&presented, "Leave?")
                    .action(DialogAction::cancel("Stay", move |_: Environment| {
                        cancelled.set(true);
                    }))
                    .action(DialogAction::new("Leave", || {})),
            )
        });
    app.settle();
    shown.set(true);
    app.settle();

    app.tap_at(8.0, 8.0);

    assert!(hit.get(), "the scrim tap did not reach the cancel handler");
    assert!(!shown.snapshot());
}

/// Two dialogs on one window, the later-declared one presented first.
fn two_dialogs(first: &Binding<bool>, second: &Binding<bool>) -> impl waterui::View + use<> {
    vstack((
        text("Top").dialog(
            Dialog::new(second, "Second dialog")
                .action(DialogAction::cancel("Dismiss second", || {})),
        ),
        text("Bottom").dialog(
            Dialog::new(first, "First dialog").action(DialogAction::cancel("Dismiss first", || {})),
        ),
    ))
}

fn assert_second_dialog_waits<R: RuntimeDriver>(
    app: &mut SemanticApp<R>,
    first: &Binding<bool>,
    second: &Binding<bool>,
) {
    first.set(true);
    app.settle();
    app.query().label("First dialog").assert_exists();

    second.set(true);
    app.settle();
    app.query().label("First dialog").assert_exists();
    app.query().label("Second dialog").assert_not_exists();

    app.query()
        .role(Role::BUTTON)
        .label("Dismiss first")
        .single()
        .tap(app);
    app.settle();
    assert!(!first.snapshot());
    assert!(
        second.snapshot(),
        "the waiting dialog lost its presentation"
    );
    app.query().label("Second dialog").assert_exists();
}

#[test]
fn a_second_dialog_waits_in_presentation_order_on_semantic_mount() {
    let first = Binding::bool(false);
    let second = Binding::bool(false);
    let (f, s) = (first.clone(), second.clone());
    let mut app = ui().viewport(400, 300).mount(move || two_dialogs(&f, &s));
    app.settle();
    assert_second_dialog_waits(&mut app, &first, &second);
}

#[test]
fn a_second_dialog_waits_in_presentation_order_on_offscreen_mount() {
    let first = Binding::bool(false);
    let second = Binding::bool(false);
    let (f, s) = (first.clone(), second.clone());
    let mut app = ui()
        .viewport(400, 300)
        .theme(Material3::defaults())
        .mount_offscreen(move || two_dialogs(&f, &s));
    app.settle();
    assert_second_dialog_waits(&mut app, &first, &second);
}

#[test]
fn focus_moves_into_the_dialog_and_returns_on_dismissal() {
    let presented = Binding::bool(false);
    let flag = Rc::new(Cell::new(false));
    let shown = presented.clone();
    let mut app = ui()
        .viewport(400, 300)
        .mount(move || fixture(&presented, flag.clone()));
    app.settle();
    app.press_named_key("Tab");
    app.settle();
    app.query()
        .role(Role::BUTTON)
        .label("Open")
        .assert_ui_focus();

    shown.set(true);
    app.settle();
    app.query()
        .role(Role::BUTTON)
        .label("Cancel")
        .assert_ui_focus();

    app.press_named_key("Escape");
    app.settle();
    assert!(!shown.snapshot());
    app.query()
        .role(Role::BUTTON)
        .label("Open")
        .assert_ui_focus();
}
