//! `OffscreenApp::settle` runs finite work to its end.

use super::*;
use waterui::ViewExt;
use waterui::animation::Animation;
use waterui::layout::scroll::{ScrollController, scroll};

/// A fixed-size row, so the content's extent is set by the test, not a style.
fn card(label: &'static str) -> impl View {
    ViewExt::size(text(label), 120.0, 48.0)
}

/// A spring's settling duration is physical — `Animation::spring(100, 10)`
/// runs past a second — so `settle` has to run it to its end rather than
/// return at a fixed budget with the offset still in flight.
#[test]
fn settle_runs_a_spring_scroll_to_its_target() {
    let offset = waterui::binding(Point::zero());
    let controller = ScrollController::new(Point::zero());
    let mut app = ui()
        .viewport(180, 180)
        .theme(TestStyle::new())
        .mount_offscreen({
            let offset = offset.clone();
            let controller = controller.clone();
            move || {
                scroll(
                    vstack((card("First"), card("Second"), card("Third"), card("Fourth")))
                        .spacing(12.0),
                )
                .report_offset(&offset)
                .scroll_controller(&controller)
                .size(120.0, 120.0)
            }
        });
    app.settle();

    let spring = Animation::spring(100.0, 10.0);
    controller.animate_to(Point::new(0.0, 60.0), spring);
    app.settle();
    assert_eq!(
        offset.snapshot(),
        Point::new(0.0, 60.0),
        "settle must return only once the spring scroll lands on its target"
    );
}

/// A focused field's caret blinks for as long as it holds focus: the blink
/// repaints pixels without moving semantic state, so it is never finite
/// work, and settling a scene with a focused field returns.
#[test]
fn settle_returns_with_a_focused_field() {
    let value = waterui::binding(waterui::Str::from(""));
    let mut app = ui()
        .viewport(320, 120)
        .theme(TestStyle::new())
        .mount_offscreen(move || waterui::component::field("Email", &value).size(280.0, 56.0));
    let field = app.query().role(Role::TEXT_INPUT).label("Email").single();
    app.query().role(Role::TEXT_INPUT).label("Email").focus();
    app.settle();
    assert_eq!(
        app.tree().focus(),
        field.id(),
        "the field keeps focus while the scene settles"
    );
}
