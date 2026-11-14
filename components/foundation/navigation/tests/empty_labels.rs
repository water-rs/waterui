//! A navigation page that binds no subtitle emits no subtitle node —
//! empty `Label` nodes stay out of the accessibility tree
//! (water-rs/hydrolysis#176; #167 introduced the subtitle node and started
//! realizing one on every page).

use waterui::component::text;
use waterui_navigation::NavigationView;
use waterui_testing::{Role, UiBuilder};

/// The issue's shape: a titled navigation page that binds no subtitle.
fn inbox_view() -> NavigationView {
    NavigationView::new("Inbox", text("mail body"))
}

#[waterui::test(viewport = (390, 844))]
fn navigation_without_subtitle_emits_no_subtitle_node(ui: UiBuilder) {
    let mut app = ui.mount(inbox_view);
    app.settle();
    let bar = app.query().role(Role::NAVIGATION).single();
    app.query()
        .role(Role::HEADER)
        .label("Inbox")
        .children_of(&bar)
        .assert_exists();
    assert!(
        !app.query().role(Role::LABEL).children_of(&bar).exists(),
        "an unbound subtitle must emit no Label child under the bar node"
    );
}
