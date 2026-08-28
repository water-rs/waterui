//! Visual acceptance for Material's loading indicator.
//!
//! The indicator morphs through seven shapes while it turns, so a single frame
//! is a sample of that motion. This captures several phases across one morph
//! interval, which is what makes a stuck or mis-scaled shape visible.

use core::time::Duration;
use waterui::prelude::*;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

fn indicator() -> impl View {
    vstack((waterui::component::progress::loading().label("Loading"),)).padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (120, 120))]
fn the_loading_indicator_morphs_as_it_turns(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(indicator);

    // Sample across one morph interval (650ms). Pumping is what advances the
    // animation clock — wall-clock sleeping would freeze it and every capture
    // would show the same frame.
    for step in 0..4 {
        app.pump_for(Duration::from_millis(160));
        let _ = app.capture_snapshot(
            "material3-preview",
            "loading-indicator",
            format!("phase-{step}"),
        );
    }
}

/// The indicator is an indeterminate progress indicator, and says so.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (120, 120))]
fn the_loading_indicator_exposes_progress_semantics(ui: UiBuilder) {
    let mut app = ui.mount(indicator);

    app.query()
        .role(Role::PROGRESS_INDICATOR)
        .label("Loading")
        .assert_exists();
}
