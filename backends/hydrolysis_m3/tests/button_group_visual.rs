//! Visual acceptance for the Material 3 connected button group.
//!
//! Only the ends of the group are fully round; the corners either side of each
//! seam are tucked in, and the selected segment rounds back out so it reads as
//! lifted out of the row. The PNG is reviewed by eye.

use hydrolysis_m3::{connected_button, connected_button_group};
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
