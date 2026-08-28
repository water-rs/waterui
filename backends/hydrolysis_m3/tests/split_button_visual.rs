//! Visual acceptance for the Material 3 split button.
//!
//! The pair must read as one control with a seam: fully round on the outside,
//! tucked in where the halves meet. The PNG is reviewed by eye.

use hydrolysis_m3::split_button;
use waterui::prelude::*;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

fn buttons() -> impl View {
    vstack((
        split_button("Save", "More save options"),
        split_button("Publish", "More publish options").tonal(),
    ))
    .spacing(16.0)
    .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 160))]
fn split_button_renders_as_one_silhouette(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(buttons);
    let _ = app.capture_snapshot("material3-preview", "split-button", "filled-and-tonal");
}

/// Both halves must be reachable and separately labelled, or the trailing
/// chevron becomes an unnamed tap target.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 160))]
fn split_button_exposes_both_halves(ui: UiBuilder) {
    let mut app = ui.mount(buttons);

    app.query().role(Role::BUTTON).label("Save").assert_exists();
    app.query()
        .role(Role::BUTTON)
        .label("More save options")
        .assert_exists();
}
