use std::cell::Cell;
use std::rc::Rc;

use hydrolysis_m3::Material3;
use waterui::ViewExt as _;
use waterui::component::{button, text, vstack};
use waterui::reactive::{Signal as _, binding};
use waterui::sheet::{Sheet, SheetDetent, SheetDismiss};

use crate::{Role, ui};

#[test]
fn sheet_makes_underlying_content_inert_and_dismiss_handle_closes() {
    let open = binding(false);
    let builds = Rc::new(Cell::new(0_u32));
    let dismiss_calls = Rc::new(Cell::new(0_u32));

    let open_view = open.clone();
    let builds_view = Rc::clone(&builds);
    let dismiss_view = Rc::clone(&dismiss_calls);

    let mut app = ui().mount(move || {
        let builds_sheet = Rc::clone(&builds_view);
        let dismiss_sheet = Rc::clone(&dismiss_view);
        vstack((button("Underneath").action(|| {}), text("Workspace"))).sheet(
            Sheet::new(&open_view, move || {
                builds_sheet.set(builds_sheet.get() + 1);
                vstack((
                    text("Sheet Body"),
                    button("Done").action(|sheet: SheetDismiss| sheet.dismiss()),
                ))
            })
            .label("Settings Sheet")
            .on_dismiss(move || dismiss_sheet.set(dismiss_sheet.get() + 1)),
        )
    });
    app.settle();

    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_exists();
    assert_eq!(
        builds.get(),
        0,
        "sheet content must not build while dismissed"
    );

    open.set(true);
    app.settle();
    assert_eq!(builds.get(), 1);
    app.query()
        .role(Role::DIALOG)
        .label("Settings Sheet")
        .assert_exists();
    app.query().role(Role::BUTTON).label("Done").assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_not_exists();

    app.query().role(Role::BUTTON).label("Done").tap();
    app.settle();
    assert!(!open.snapshot());
    assert_eq!(
        dismiss_calls.get(),
        0,
        "SheetDismiss::dismiss closes programmatically without running on_dismiss"
    );
    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("Done")
        .assert_not_exists();

    // Re-presenting invokes the builder freshly.
    open.set(true);
    app.settle();
    assert_eq!(builds.get(), 2);
}

#[test]
fn sheet_contains_focus_and_escape_restores_focus_and_runs_on_dismiss() {
    let open = binding(false);
    let underneath_taps = Rc::new(Cell::new(0_u32));
    let sheet_taps = Rc::new(Cell::new(0_u32));
    let dismiss_calls = Rc::new(Cell::new(0_u32));

    let open_view = open.clone();
    let underneath_view = Rc::clone(&underneath_taps);
    let sheet_view = Rc::clone(&sheet_taps);
    let dismiss_view = Rc::clone(&dismiss_calls);

    let mut app = ui().mount(move || {
        let underneath = Rc::clone(&underneath_view);
        let sheet_btn = Rc::clone(&sheet_view);
        let dismiss_cb = Rc::clone(&dismiss_view);
        vstack((
            button("Underneath").action(move || underneath.set(underneath.get() + 1)),
            button("Secondary").action(|| {}),
        ))
        .sheet(
            Sheet::new(&open_view, move || {
                let sheet_btn = Rc::clone(&sheet_btn);
                button("Inside Sheet").action(move || sheet_btn.set(sheet_btn.get() + 1))
            })
            .on_dismiss(move || dismiss_cb.set(dismiss_cb.get() + 1)),
        )
    });
    app.settle();

    // Focus "Underneath" before presenting the sheet.
    app.press_named_key("Tab");
    app.press_named_key("Enter");
    app.settle();
    assert_eq!(underneath_taps.get(), 1);

    // Open the sheet: focus moves into the sheet and Tab stays inside it.
    open.set(true);
    app.settle();
    app.press_named_key("Enter");
    app.settle();
    assert_eq!(sheet_taps.get(), 1);
    assert_eq!(underneath_taps.get(), 1);

    // Tabbing repeatedly inside the sheet must never escape to underlying controls.
    app.press_named_key("Tab");
    app.press_named_key("Enter");
    app.settle();
    assert_eq!(sheet_taps.get(), 2);
    assert_eq!(underneath_taps.get(), 1);

    // Escape invokes on_dismiss, closes the sheet, and restores focus to "Underneath".
    app.press_named_key("Escape");
    app.settle();
    assert!(!open.snapshot());
    assert_eq!(dismiss_calls.get(), 1);
    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_exists();

    app.press_named_key("Enter");
    app.settle();
    assert_eq!(
        underneath_taps.get(),
        2,
        "dismissing the sheet must restore keyboard focus to the previously focused control"
    );
}

#[test]
fn rendered_sheet_blocks_underlying_pointer_taps_and_scrim_dismisses() {
    let open = binding(false);
    let underneath_taps = Rc::new(Cell::new(0_u32));
    let sheet_action_taps = Rc::new(Cell::new(0_u32));
    let dismiss_calls = Rc::new(Cell::new(0_u32));

    let open_view = open.clone();
    let underneath_view = Rc::clone(&underneath_taps);
    let sheet_action_view = Rc::clone(&sheet_action_taps);
    let dismiss_view = Rc::clone(&dismiss_calls);

    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(400, 600)
        .mount_offscreen(move || {
            let underneath = Rc::clone(&underneath_view);
            let sheet_action = Rc::clone(&sheet_action_view);
            let dismiss_cb = Rc::clone(&dismiss_view);
            vstack((
                button("Underneath").action(move || underneath.set(underneath.get() + 1)),
                text("Main content"),
            ))
            .sheet(
                Sheet::new(&open_view, move || {
                    let sheet_action = Rc::clone(&sheet_action);
                    vstack((
                        text("Medium Sheet"),
                        button("Sheet Action")
                            .action(move || sheet_action.set(sheet_action.get() + 1)),
                    ))
                })
                .detents([SheetDetent::Medium])
                .on_dismiss(move || dismiss_cb.set(dismiss_cb.get() + 1)),
            )
        });
    app.settle();

    let under_bounds = app
        .query()
        .role(Role::BUTTON)
        .label("Underneath")
        .single()
        .bounds();
    let tap_x = under_bounds.width().mul_add(0.5, under_bounds.x());
    let tap_y = under_bounds.height().mul_add(0.5, under_bounds.y());

    open.set(true);
    app.settle();
    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_not_exists();
    let dialog_bounds = app.query().role(Role::DIALOG).single().bounds();
    assert!(
        (dialog_bounds.width() - 400.0).abs() < 1.0
            && (dialog_bounds.height() - 300.0).abs() < 1.0
            && (dialog_bounds.y() - 300.0).abs() < 1.0,
        "medium detent sheet should fill bottom 400x300 detent rect, got {dialog_bounds:?}"
    );

    // Tapping non-interactive text and empty surface inside the sheet must not
    // fall through to the scrim behind it.
    let label_bounds = app
        .query()
        .role(Role::LABEL)
        .label("Medium Sheet")
        .single()
        .bounds();
    app.tap_at(
        label_bounds.width().mul_add(0.5, label_bounds.x()),
        label_bounds.height().mul_add(0.5, label_bounds.y()),
    );
    app.tap_at(200.0, 560.0);
    app.settle();
    assert!(
        open.snapshot(),
        "tapping inside the sheet surface must not dismiss it"
    );
    assert_eq!(dismiss_calls.get(), 0);

    // Interactive controls inside the sheet still receive pointer taps.
    app.query().role(Role::BUTTON).label("Sheet Action").tap();
    app.settle();
    assert_eq!(sheet_action_taps.get(), 1);
    assert!(open.snapshot());
    assert_eq!(dismiss_calls.get(), 0);

    // Tapping where "Underneath" sits hits the modal scrim instead of the button.
    app.tap_at(tap_x, tap_y);
    app.settle();
    assert_eq!(
        underneath_taps.get(),
        0,
        "pointer tap over the scrim must never reach content beneath the sheet"
    );
    assert!(!open.snapshot());
    assert_eq!(dismiss_calls.get(), 1);
}

#[test]
fn non_dismissible_full_screen_sheet_ignores_escape_and_scrim_taps() {
    let open = binding(true);
    let underneath_taps = Rc::new(Cell::new(0_u32));
    let dismiss_calls = Rc::new(Cell::new(0_u32));

    let open_view = open.clone();
    let underneath_view = Rc::clone(&underneath_taps);
    let dismiss_view = Rc::clone(&dismiss_calls);

    let mut app = ui()
        .theme(Material3::defaults())
        .viewport(400, 600)
        .mount_offscreen(move || {
            let underneath = Rc::clone(&underneath_view);
            let dismiss_cb = Rc::clone(&dismiss_view);
            button("Underneath")
                .action(move || underneath.set(underneath.get() + 1))
                .sheet(
                    Sheet::new(&open_view, || text("Loading..."))
                        .detents([SheetDetent::FullScreen])
                        .dismissible(false)
                        .on_dismiss(move || dismiss_cb.set(dismiss_cb.get() + 1)),
                )
        });
    app.settle();

    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_not_exists();
    app.query()
        .role(Role::LABEL)
        .label("Loading...")
        .assert_exists();

    app.press_named_key("Escape");
    app.tap_at(200.0, 300.0);
    app.settle();
    assert!(open.snapshot());
    assert_eq!(dismiss_calls.get(), 0);
    assert_eq!(underneath_taps.get(), 0);

    open.set(false);
    app.settle();
    app.query()
        .role(Role::BUTTON)
        .label("Underneath")
        .assert_exists();
}
