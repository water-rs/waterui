//! The snackbar container hugs its content between the theme's width bounds.
//!
//! A closeable bar used to stretch to the theme's `max_width` no matter how
//! short its message was, because `max_width` on a `Frame` carries `SwiftUI`'s
//! expansion semantics. The bar now sizes to its row clamped into
//! `[min_width, max_width]`, and the spacer inside the row pins the trailing
//! controls to the trailing edge whenever the `min_width` floor leaves slack.

use std::time::Duration;

use hydrolysis_m3::install as install_m3;
use waterui::Environment;
use waterui::component::zstack;
use waterui::layout::padding::EdgeInsets;
use waterui::layout::safe_area::SafeAreaInsets;
use waterui::prelude::*;
use waterui::snackbar::{Snackbar, SnackbarManager, SnackbarPosition, SnackbarTheme};
use waterui_testing::UiBuilder;

/// The width bounds these tests pin the bar to, distinctive on purpose so an
/// ambient theme silently overriding them shows up as a failed assertion.
const MIN_WIDTH: f32 = 300.0;
const MAX_WIDTH: f32 = 600.0;

/// Mounts an app whose window shows `snackbar`, and returns it settled.
fn show(ui: UiBuilder, snackbar: Snackbar) -> (waterui_testing::SemanticApp, SnackbarManager) {
    let (manager, overlay) = SnackbarManager::new();
    let theme = SnackbarTheme {
        min_width: MIN_WIDTH,
        max_width: MAX_WIDTH,
        ..SnackbarTheme::default()
    };
    let mut env = Environment::new();
    env.insert(manager.clone());
    env.insert(theme);
    let mut app = ui
        .theme(install_m3)
        .environment(env)
        .mount(move || zstack((text("app content"), overlay.clone())));
    manager.show(snackbar);
    app.settle();
    (app, manager)
}

/// A closeable bar with a short message sits at the `min_width` floor with its
/// close control pinned to the trailing edge — nowhere near the `max_width`
/// cap it used to stretch to.
#[waterui::test(viewport = (900, 600))]
fn a_short_closeable_snackbar_hugs_the_min_width_floor(ui: UiBuilder) {
    let (mut app, _manager) = show(
        ui,
        Snackbar::new("Saved").duration(Duration::ZERO).closeable(),
    );

    let message = app.query().label("Saved").single().bounds();
    let close = app.query().label("Close").single().bounds();

    let span = close.x() + close.width() - message.x();
    assert!(
        span <= MIN_WIDTH,
        "the bar's content spans {span} logical pixels, wider than the \
         {MIN_WIDTH} floor — it is still stretching toward the {MAX_WIDTH} cap"
    );
    assert!(
        span > MIN_WIDTH * 0.7,
        "the close control sits {span} from the message's leading edge; the \
         spacer should pin it near the {MIN_WIDTH} floor's trailing edge"
    );
}

/// A plain bar hugs its message rather than stretching to the cap.
#[waterui::test(viewport = (900, 600))]
fn a_plain_snackbar_hugs_its_message(ui: UiBuilder) {
    let (mut app, _manager) = show(
        ui,
        Snackbar::new("Copied to clipboard").duration(Duration::ZERO),
    );

    let message = app.query().label("Copied to clipboard").single().bounds();
    assert!(
        message.width() < MIN_WIDTH,
        "a short message stays narrower than the bar's own floor"
    );
}

/// A top-positioned bar clears the device's top inset.
///
/// The theme's `viewport_padding` is a margin, not a stand-in for hardware:
/// a bar that only honoured it landed underneath the notch on every phone with
/// one. The backend publishes the window's safe area, and the bar pads by the
/// inset *plus* the margin.
#[waterui::test(viewport = (900, 600))]
fn a_top_snackbar_clears_the_published_safe_area(ui: UiBuilder) {
    const TOP_INSET: f32 = 59.0;

    let (manager, overlay) = SnackbarManager::new();
    let theme = SnackbarTheme {
        viewport_padding: EdgeInsets::all(16.0),
        ..SnackbarTheme::default()
    };
    let mut env = Environment::new();
    env.insert(manager.clone());
    env.insert(theme);
    SafeAreaInsets::install(&mut env, EdgeInsets::new(TOP_INSET, 34.0, 0.0, 0.0));

    let mut app = ui
        .theme(install_m3)
        .environment(env)
        .mount(move || zstack((text("app content"), overlay.clone())));
    manager.show(
        Snackbar::new("Uploaded")
            .duration(Duration::ZERO)
            .position(SnackbarPosition::TopCenter),
    );
    app.settle();

    let message = app.query().label("Uploaded").single().bounds();
    assert!(
        message.y() >= TOP_INSET,
        "the bar's message starts at y={}, inside the {TOP_INSET}pt top inset — \
         it is sitting under the notch",
        message.y()
    );
}
