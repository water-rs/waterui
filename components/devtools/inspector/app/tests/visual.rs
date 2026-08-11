//! Visual acceptance for the Inspector's panes.
//!
//! Each pane is mounted against the same populated [`Model`] the previews use,
//! so what is reviewed is the real view code driven by real protocol types
//! rather than a mock. The PNG-producing tests are ignored by default and
//! reviewed by eye.

use hydrolysis_m3::install;
use waterui_inspector_app::model::Section;
use waterui_inspector_app::{connection, preview_model, ui};
use waterui_testing::{OffscreenApp, ui as test_ui};

fn save(app: &mut OffscreenApp, case: &str) {
    let _ = app.capture_snapshot("inspector", case, "default");
}

/// Mounts one pane at a size that shows its whole layout.
fn pane(case: &str, width: u32, height: u32, section: Section) {
    let mut app = test_ui()
        .viewport(width, height)
        .theme(install)
        .mount(move || {
            let model = preview_model();
            let (sender, _receiver) = connection::subscription_channel();
            ui::pane(section, model, sender)
        });
    save(&mut app, case);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn overview_pane() {
    pane("overview", 900, 700, Section::Overview);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn frames_pane() {
    pane("frames", 900, 700, Section::Frames);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn tree_pane() {
    pane("tree", 900, 700, Section::Tree);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn tasks_pane() {
    pane("tasks", 900, 700, Section::Tasks);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn logs_pane() {
    pane("logs", 900, 700, Section::Logs);
}

#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn signals_pane() {
    pane("signals", 900, 700, Section::Signals);
}

/// The whole window, sidebar included.
#[test]
#[ignore = "writes visual acceptance PNG files for direct image review"]
fn full_window() {
    let mut app = test_ui()
        .viewport(1200, 800)
        .theme(install)
        .mount(move || {
            let model = preview_model();
            let (sender, _receiver) = connection::subscription_channel();
            ui::inspector(model, sender)
        });
    save(&mut app, "window");
}
