//! Right-clicking anything in a debug build offers to inspect it.
//!
//! Reached the way a user reaches it: press the secondary button and read the
//! menu. The menu opens in its own window, so this also exercises the harness
//! seeing more than the main one.

use hydrolysis_m3::install;
use waterui::prelude::*;
use waterui_testing::ui as test_ui;

/// A plain view, with no context menu of its own, still offers to be inspected.
#[core::prelude::v1::test]
fn right_clicking_offers_to_inspect_the_element() {
    let mut app = test_ui()
        .viewport(320, 200)
        .theme(install)
        .mount(|| text("Hello").padding_with(EdgeInsets::all(20.0)));

    app.query().label("Inspect element").assert_not_exists();

    app.secondary_click_at(40.0, 30.0);

    app.query().label("Inspect element").assert_exists();
}

/// An application's own menu keeps its items: the inspector entry is appended
/// after what the application put there, not instead of it.
#[core::prelude::v1::test]
fn an_application_menu_keeps_its_own_items() {
    let mut app = test_ui().viewport(320, 200).theme(install).mount(|| {
        text("Hello")
            .padding_with(EdgeInsets::all(20.0))
            .context_menu(vec!["Rename".action(|| {})])
    });

    app.secondary_click_at(40.0, 30.0);

    app.query().label("Rename").assert_exists();
    app.query().label("Inspect element").assert_exists();
}
