//! Visual acceptance for the Material 3 floating action button menu.
//!
//! The items must reveal from the bottom up, so a frame captured mid-expansion
//! shows the lower items ahead of the upper ones. The PNGs are reviewed by eye.

use core::time::Duration;

use hydrolysis_m3::{fab_menu, fab_menu_item};
use waterui::prelude::theme_color::Foreground;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{Circle, ShapeExt as _};
use waterui_testing::{OffscreenApp, Role, UiBuilder};

#[allow(
    clippy::needless_pass_by_value,
    reason = "the view outlives the call, so it cannot borrow the binding"
)]
fn menu(expanded: Binding<bool>) -> impl View {
    let dot = || Circle.fill(Foreground);
    fab_menu(&expanded, "More actions", dot())
        .item(fab_menu_item("Share", dot(), || {}))
        .item(fab_menu_item("Duplicate", dot(), || {}))
        .item(fab_menu_item("Archive", dot(), || {}))
        .padding()
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 320))]
fn fab_menu_reveals_its_items_from_the_bottom_up(ui: UiBuilder) {
    let expanded = binding(false);
    let toggle = expanded.clone();
    let mut app: OffscreenApp = ui.mount_offscreen(move || menu(expanded.clone()));

    let _ = app.capture_snapshot("material3-preview", "fab-menu", "collapsed");
    toggle.set(true);
    app.pump_for(Duration::from_millis(120));
    let _ = app.capture_snapshot("material3-preview", "fab-menu", "expanding-120ms");
    app.pump_for(Duration::from_millis(600));
    let _ = app.capture_snapshot("material3-preview", "fab-menu", "expanded");
}

/// Every action must be reachable and named once the menu is open.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 320))]
fn fab_menu_exposes_its_actions(ui: UiBuilder) {
    let expanded = binding(true);
    let mut app = ui.mount(move || menu(expanded.clone()));

    app.query()
        .role(Role::BUTTON)
        .label("More actions")
        .assert_exists();
    for action in ["Share", "Duplicate", "Archive"] {
        app.query().label(action).assert_exists();
    }
}
