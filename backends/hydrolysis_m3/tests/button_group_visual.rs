//! Visual acceptance for the Material 3 connected button group.
//!
//! Only the ends of the group are fully round; the corners either side of each
//! seam are tucked in, and the selected segment rounds back out so it reads as
//! lifted out of the row. The PNG is reviewed by eye.

use hydrolysis_m3::{connected_button, connected_button_group};
use waterui::prelude::vstack;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

fn group() -> impl View {
    let day = binding(false);
    let week = binding(true);
    let month = binding(false);
    vstack((connected_button_group()
        .segment(connected_button("Day", &day, || {}))
        .segment(connected_button("Week", &week, || {}))
        .segment(connected_button("Month", &month, || {})),))
    .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (360, 120))]
fn connected_group_tucks_its_inner_corners(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(group);
    let _ = app.capture_snapshot(
        "material3-preview",
        "connected-button-group",
        "selected-middle",
    );
}

/// Each segment is a button carrying its own selected state, so assistive
/// technology can tell which choice is active.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (360, 120))]
fn connected_group_exposes_each_segment(ui: UiBuilder) {
    let mut app = ui.mount(group);

    for segment in ["Day", "Week", "Month"] {
        app.query()
            .role(Role::BUTTON)
            .label(segment)
            .assert_exists();
    }
    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("Week")
            .single()
            .node()
            .selected(),
        "the active choice should report itself selected"
    );
}

/// The spaced group: three buttons that keep their own shape, with the gap
/// between them coming from the layout rather than from padding.
fn spaced_group() -> impl View {
    vstack((hydrolysis_m3::button_group()
        .button(hydrolysis_m3::group_button("Rewind", || {}))
        .button(hydrolysis_m3::group_button("Play", || {}))
        .button(hydrolysis_m3::group_button("Forward", || {})),))
    .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (420, 120))]
fn spaced_group_lays_its_buttons_out_in_a_row(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(spaced_group);
    let _ = app.capture_snapshot("material3-preview", "button-group", "spaced");
}

/// Every button in the group is its own target, and the group announces itself
/// as the set they belong to.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (420, 120))]
fn spaced_group_exposes_every_button(ui: UiBuilder) {
    let mut app = ui.mount(spaced_group);

    for button in ["Rewind", "Play", "Forward"] {
        app.query().role(Role::BUTTON).label(button).assert_exists();
    }
}

/// The buttons sit side by side, in order, separated by the group's between
/// space — the layout places them, so this is what proves it ran.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (420, 120))]
fn spaced_group_places_its_buttons_in_order(ui: UiBuilder) {
    let mut app = ui.mount(spaced_group);

    let mut bounds = |label: &str| {
        app.query()
            .role(Role::BUTTON)
            .label(label)
            .single()
            .node()
            .bounds()
            .expect("a placed button has bounds")
    };
    let mut span = |label: &str| {
        let bounds = bounds(label);
        (bounds.x(), bounds.x() + bounds.width())
    };
    let (rewind_left, rewind_right) = span("Rewind");
    let (play_left, play_right) = span("Play");
    let (forward_left, _) = span("Forward");

    assert!(
        rewind_right <= play_left && play_right <= forward_left,
        "buttons should be laid out left to right without overlapping: \
         {rewind_left}..{rewind_right}, {play_left}..{play_right}, {forward_left}.."
    );
    assert!(
        (play_left - rewind_right - 12.0).abs() < 1.0,
        "the gap should be ButtonGroupSmallTokens.BetweenSpace (12dp), was {}",
        play_left - rewind_right
    );
}
