//! Integration coverage for `waterui_testing::ElementRef` and `within(..)`
//! scope lifetime across tree mutations (water-rs/waterui#4321): handles stay
//! valid while their `NodeId` remains in the tree and fail loudly once the
//! node leaves the tree.

use std::any::Any;

use waterui::prelude::*;
use waterui_testing::{Role, ui};

fn panic_message(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<&'static str>()
        .map(|s| (*s).to_owned())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| String::from("<non-string panic payload>"))
}

#[test]
fn element_handle_and_scope_survive_unrelated_updates_and_panic_on_removal() {
    let show_target: Binding<bool> = binding(true);
    let extra_rows: Binding<usize> = binding(0);
    let target_taps: Binding<usize> = binding(0);

    let show_target_view = show_target.clone();
    let target_taps_view = target_taps.clone();

    let mut app = ui().mount(move || {
        let extra_rows_for_add = extra_rows.clone();
        let extra_rows_for_label = extra_rows.clone();
        let target_taps_for_btn = target_taps_view.clone();
        let show_target_for_when = show_target_view.clone();

        vstack((
            button("Add").action(move || {
                extra_rows_for_add.set(extra_rows_for_add.snapshot() + 1);
            }),
            text(
                extra_rows_for_label
                    .map(|count| format!("Added {count}"))
                    .computed(),
            ),
            when(show_target_for_when, move || {
                let taps = target_taps_for_btn.clone();
                vstack((
                    button("Target").action(move || {
                        taps.set(taps.snapshot() + 1);
                    }),
                    text("Scoped child"),
                ))
                .a11y_role(waterui::accessibility::AccessibilityRole::Group)
                .a11y_label("Target panel")
            }),
        ))
    });

    let panel = app.query().role(Role::GROUP).label("Target panel").single();
    let target = app
        .query()
        .within(&panel)
        .role(Role::BUTTON)
        .label("Target")
        .single();

    let initial_revision = app.tree().revision();

    // Unrelated tree update advances the revision, yet `panel` and `target` stay in the tree.
    app.query().role(Role::BUTTON).label("Add").tap();
    app.query()
        .role(Role::LABEL)
        .label("Added 1")
        .assert_exists();
    assert!(
        app.tree().revision() > initial_revision,
        "unrelated mutation must advance tree revision"
    );

    target.tap(&mut app);
    assert_eq!(target_taps.snapshot(), 1);
    app.query()
        .within(&panel)
        .role(Role::LABEL)
        .label("Scoped child")
        .assert_exists();

    // Removing the nodes from the tree invalidates both direct interaction and scoped query.
    show_target.set(false);
    app.query()
        .role(Role::BUTTON)
        .label("Target")
        .assert_not_exists();

    let tap_err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        target.tap(&mut app);
    }))
    .expect_err("tapping a removed element handle must panic");
    let tap_msg = panic_message(&*tap_err);
    assert!(
        tap_msg.contains("no longer exists in tree"),
        "unexpected panic message for removed handle tap: {tap_msg}"
    );

    let scope_err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = app
            .query()
            .within(&panel)
            .role(Role::LABEL)
            .label("Scoped child")
            .all();
    }))
    .expect_err("querying within a removed scope handle must panic");
    let scope_msg = panic_message(&*scope_err);
    assert!(
        scope_msg.contains("no longer exists in tree"),
        "unexpected panic message for removed scope query: {scope_msg}"
    );
}
