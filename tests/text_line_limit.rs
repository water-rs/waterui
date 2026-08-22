//! `Text::line_limit` and the single-line button-label baseline.
//!
//! `SwiftUI` and Material keep a button's label on one truncated line; plain
//! text wraps freely. Both halves are asserted through the accessibility
//! tree's bounds, which is how the headless renderer reports layout.

use hydrolysis_m3::install as install_m3;
use waterui::ViewExt as _;
use waterui::component::{hstack, vstack};
use waterui::prelude::*;
use waterui_testing::UiBuilder;

const LONG: &str = "A label long enough to wrap into several lines at this width";

/// A limited text reserves height for its visible lines only.
#[waterui::test(viewport = (600, 600), theme = install_m3)]
fn a_line_limit_caps_the_reserved_height(ui: UiBuilder) {
    let mut app = ui.mount(|| {
        vstack((
            text(LONG).a11y_label("unlimited").width(150.0),
            text(LONG)
                .line_limit(core::num::NonZeroUsize::MIN)
                .a11y_label("limited")
                .width(150.0),
        ))
    });

    let unlimited = app.query().label("unlimited").single().bounds();
    let limited = app.query().label("limited").single().bounds();
    assert!(
        unlimited.height() > limited.height() * 2.0,
        "the unlimited text wraps ({}) while the limited one stays one line ({})",
        unlimited.height(),
        limited.height(),
    );
}

/// Compressed buttons keep their labels on one line instead of folding them
/// into paragraphs — the webview example's toolbar rendered "Back" as
/// "Bac / k" before button labels defaulted to a single truncated line.
#[waterui::test(viewport = (600, 600), theme = install_m3)]
fn compressed_button_labels_stay_on_one_line(ui: UiBuilder) {
    let mut app = ui.mount(|| {
        hstack((
            button("Back").action(|| {}),
            button("Forward").action(|| {}),
            button("Reload").action(|| {}),
            button("Stop").action(|| {}),
        ))
        .width(250.0)
    });

    let mut heights = Vec::new();
    for label in ["Back", "Forward", "Reload", "Stop"] {
        heights.push(app.query().label(label).single().bounds().height());
    }
    let min = heights.iter().copied().fold(f32::INFINITY, f32::min);
    let max = heights.iter().copied().fold(0.0_f32, f32::max);
    assert!(
        (max - min) < 1.0,
        "buttons disagree about line count, so a label folded: {heights:?}"
    );
}
