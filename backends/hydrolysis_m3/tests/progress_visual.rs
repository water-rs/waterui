//! Visual acceptance for the Material 3 Expressive progress indicators.
//!
//! Expressive rounds both ends of the linear bar, separates the active
//! indicator from the track by `TrackActiveSpace`, and marks the track's end
//! with a stop indicator. The PNG is reviewed by eye.

use waterui::component::progress::progress;
use waterui::prelude::*;
use waterui_testing::{OffscreenApp, UiBuilder};

fn indicators() -> impl View {
    vstack((
        progress(Binding::f64(0.25)).label("Quarter"),
        progress(Binding::f64(0.5)).label("Half"),
        progress(Binding::f64(0.9)).label("Nearly done"),
        progress(Binding::f64(1.0)).label("Complete"),
    ))
    .spacing(20.0)
    .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (420, 300))]
fn linear_progress_renders_the_expressive_track_gap(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(indicators);
    let _ = app.capture_snapshot("material3-preview", "progress", "linear-expressive");
}

fn circular_indicators() -> impl View {
    hstack((
        progress(Binding::f64(0.25)).circular(),
        progress(Binding::f64(0.6)).circular(),
        progress(Binding::f64(0.95)).circular(),
    ))
    .spacing(24.0)
    .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (300, 120))]
fn circular_progress_draws_its_track(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(circular_indicators);
    let _ = app.capture_snapshot("material3-preview", "progress", "circular-expressive");
}
