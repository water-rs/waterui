//! The sidebar drives the split view's selection (water-rs/waterui#2231).
//!
//! Before the sidebar was a `List`, its rows were inert stack children: tapping
//! one never wrote `model.section`, so a collapsed split could not leave its
//! sidebar and a wide split never changed its detail column. These tests go
//! through the row itself — the pointer/a11y path a user takes — rather than
//! writing the binding from the test.

use std::time::Duration;

use waterui::env::with;
use waterui::prelude::*;
use waterui_inspector_app::{Model, connection, ui};
use waterui_inspector_protocol::ChannelSet;
use waterui_testing::{OffscreenApp, Role, Selector};

const TIMEOUT: Duration = Duration::from_secs(5);

/// The collapsed inspector, mounted on its sidebar.
fn inspector_collapsed() -> impl View {
    let model = Model::new();
    model.available.set(ChannelSet::all());
    model.section.set(None);
    let (sender, receiver) = connection::subscription_channel();
    // `with` holds the receiver in the subtree's environment for the life of
    // the mount, so the UI's subscription writes never hit a closed channel.
    with(ui::inspector(model, sender), receiver)
}

/// The wide inspector, mounted on the Overview detail.
fn inspector_wide() -> impl View {
    let model = Model::new();
    model.available.set(ChannelSet::all());
    let (sender, receiver) = connection::subscription_channel();
    with(ui::inspector(model, sender), receiver)
}

/// The navigation title of whichever detail is on screen.
fn detail_header(title: &str) -> Selector {
    Selector::default().role(Role::HEADER).label(title)
}

#[waterui::test(inspector_collapsed, theme = hydrolysis_m3::Material3::defaults(), viewport = (412, 915), offscreen)]
fn a_collapsed_sidebar_row_selects_its_section(app: &mut OffscreenApp) {
    // The sidebar is on screen, so its sections resolve as list items.
    app.query().role(Role::LIST_ITEM).label("View tree").tap();

    let view_tree = detail_header("View tree");
    assert!(
        app.wait_for_existence(&view_tree, TIMEOUT),
        "selecting the View tree row never showed its detail"
    );

    // The collapsed split's back affordance is a pointer-only target in the
    // top-left corner — it emits no accessibility node (a separate framework
    // gap), so the return to the sidebar is a coordinate tap.
    app.tap_at(24.0, 32.0);
    assert!(
        app.wait_for_nonexistence(&view_tree, TIMEOUT),
        "the detail stayed after the collapsed split's back tap"
    );

    // Back on the sidebar, a second selection works the same way.
    app.query().role(Role::LIST_ITEM).label("Overview").tap();
    assert!(
        app.wait_for_existence(&detail_header("Overview"), TIMEOUT),
        "selecting the Overview row never showed its detail"
    );
}

#[waterui::test(inspector_wide, theme = hydrolysis_m3::Material3::defaults(), viewport = (1000, 844), offscreen)]
fn a_wide_sidebar_row_switches_the_detail(app: &mut OffscreenApp) {
    // Mounted on the Overview selection, the detail column shows it.
    app.query()
        .role(Role::HEADER)
        .label("Overview")
        .assert_exists();

    app.query().role(Role::LIST_ITEM).label("Tasks").tap();
    assert!(
        app.wait_for_existence(&detail_header("Tasks"), TIMEOUT),
        "selecting the Tasks row never switched the detail column"
    );
}
