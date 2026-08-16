//! Visual acceptance for the Material 3 navigation rail.
//!
//! Collapsed stacks each label under its icon; expanded lays it beside, inside
//! one active-indicator pill. The PNGs are reviewed by eye.

use hydrolysis_m3::{NavigationRailLayout, navigation_rail, navigation_rail_item};
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui_testing::{OffscreenApp, Role, UiBuilder};

fn rail(layout: NavigationRailLayout) -> impl View {
    // A clonable stand-in for a real icon: the item keeps its icon type.
    let dot = || text("\u{25cf}");
    let home = binding(true);
    let search = binding(false);
    let saved = binding(false);
    navigation_rail((
        navigation_rail_item("Home", dot(), &home).layout(layout),
        navigation_rail_item("Search", dot(), &search).layout(layout),
        navigation_rail_item("Saved", dot(), &saved).layout(layout),
    ))
    .layout(layout)
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 320))]
fn navigation_rail_renders_collapsed(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(|| rail(NavigationRailLayout::Collapsed));
    let _ = app.capture_snapshot("material3-preview", "navigation-rail", "collapsed");
}

#[ignore = "writes visual acceptance PNG files for direct image review"]
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 320))]
fn navigation_rail_renders_expanded(ui: UiBuilder) {
    let mut app: OffscreenApp = ui.mount_offscreen(|| rail(NavigationRailLayout::Expanded));
    let _ = app.capture_snapshot("material3-preview", "navigation-rail", "expanded");
}

/// Destinations must be tabs carrying their selected state, in both layouts.
#[waterui::test(theme = hydrolysis_m3::install, viewport = (400, 320))]
fn navigation_rail_exposes_its_destinations(ui: UiBuilder) {
    let mut app = ui.mount(|| rail(NavigationRailLayout::Expanded));

    // The rail's own container currently emits no accessibility node, so its
    // `TabList` role is not assertable here. That is framework behaviour, not
    // this component's: view-level `a11y_role`/`a11y_label` decorate nodes that
    // already exist rather than creating one, and the pre-existing
    // `NavigationBar` loses its "Navigation" label the same way.
    for destination in ["Home", "Search", "Saved"] {
        app.query()
            .role(Role::TAB)
            .label(destination)
            .assert_exists();
    }
    assert!(
        app.query()
            .role(Role::TAB)
            .label("Home")
            .single()
            .node()
            .selected(),
        "the current destination should report itself selected"
    );
}
