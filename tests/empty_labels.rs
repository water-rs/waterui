//! Empty `Label` nodes stay out of the accessibility tree: an empty text
//! leaf emits nothing until its content becomes non-empty — the same
//! contract decorative graphics leaves already follow
//! (water-rs/hydrolysis#176). Semantic-mount coverage; the rendered-flush
//! twin lives in Hydrolysis's own suite.

use waterui::component::text;
use waterui::{Binding, Str};
use waterui_testing::{Role, UiBuilder};

/// An empty `Str` leaf names nothing: like a decorative graphics leaf it
/// emits no node until there is a label to speak.
#[waterui::test]
fn empty_text_leaf_emits_no_node(ui: UiBuilder) {
    let mut app = ui.mount(|| Str::from(""));
    app.settle();
    assert!(
        !app.query().role(Role::LABEL).exists(),
        "an empty text leaf must emit no Label node"
    );
}

/// A text leaf bound to empty content emits no node until the content
/// becomes non-empty.
#[waterui::test]
fn empty_text_gains_a_node_when_content_becomes_non_empty(ui: UiBuilder) {
    let note = Binding::container(String::new());
    let note_for_view = note.clone();
    let mut app = ui.mount(move || text(note_for_view.clone()));
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
