//! Empty `Label` nodes stay out of the accessibility tree: a navigation page
//! that binds no subtitle emits no subtitle node, and an empty text leaf
//! emits nothing until its content becomes non-empty — the same contract
//! decorative graphics leaves already follow (water-rs/hydrolysis#176; #167
//! introduced the subtitle node and started realizing one on every page).
//!
//! Exercises both runtimes — the headless semantic walk (`mount`) and the
//! rendered flush (`mount_offscreen`).

use hydrolysis_m3::Material3;
use waterui::component::text;
use waterui::navigation::NavigationView;
use waterui::{Binding, Str};
use waterui_testing::{Role, ui};

/// The issue's first shape: a titled navigation page that binds no subtitle.
fn inbox_view() -> NavigationView {
    NavigationView::new("Inbox", text("mail body"))
}

#[test]
fn navigation_without_subtitle_emits_no_subtitle_node_on_semantic_mount() {
    let mut app = ui().viewport(390, 844).mount(inbox_view);
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

#[test]
fn navigation_without_subtitle_emits_no_subtitle_node_on_offscreen_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(inbox_view);
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

/// An empty `Str` leaf names nothing: like a decorative graphics leaf it
/// emits no node until there is a label to speak.
#[test]
fn empty_text_leaf_emits_no_node_on_semantic_mount() {
    let mut app = ui().mount(|| Str::from(""));
    app.settle();
    assert!(
        !app.query().role(Role::LABEL).exists(),
        "an empty text leaf must emit no Label node"
    );
}

#[test]
fn empty_text_leaf_emits_no_node_on_offscreen_mount() {
    let mut app = ui()
        .viewport(390, 844)
        .theme(Material3::defaults())
        .mount_offscreen(|| Str::from(""));
    app.settle();
    assert!(
        !app.query().role(Role::LABEL).exists(),
        "an empty text leaf must emit no Label node"
    );
}

/// A text leaf bound to empty content emits no node until the content
/// becomes non-empty.
#[test]
fn empty_text_gains_a_node_when_content_becomes_non_empty() {
    let note = Binding::container(String::new());
    let note_for_view = note.clone();
    let mut app = ui().mount(move || text(note_for_view.clone()));
    app.settle();
    assert!(
        !app.query().role(Role::LABEL).exists(),
        "an empty text leaf must emit no Label node"
    );

    note.set("profile note".to_owned());
    app.settle();
    app.query()
        .role(Role::LABEL)
        .label("profile note")
        .assert_exists();
}
