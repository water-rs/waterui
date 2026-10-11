//! Coverage for water-rs/waterui#1874: a `TextField` declaring
//! `.content_type(..)` carries it through the merged accessibility publish —
//! the map beside the tree, not a node property — so
//! `NodeSnapshot::content_type` reads it on the semantic walk (`mount`) and
//! the rendered flush (`mount_offscreen`) alike, and `content_type`
//! selectors match on it. A field that declares none reads `None`.

use hydrolysis_m3::Material3;
use waterui::component::text_field::{ContentType, KeyboardType};
use waterui::component::vstack;
use waterui::{Binding, Str, View};
use waterui_testing::{ContentType as SnapshotContentType, Role, ui};

/// A declared one-time-code field beside an undeclared one.
fn autofill_view() -> impl View {
    let code = Binding::container(Str::from(""));
    let note = Binding::container(Str::from(""));
    vstack((
        waterui_controls::TextField::new("Verification code", &code)
            .keyboard(KeyboardType::Number)
            .content_type(ContentType::OneTimeCode),
        waterui_controls::field("Note", &note),
    ))
}

#[test]
fn text_field_content_type_reaches_the_snapshot_on_semantic_mount() {
    let mut app = ui().mount(autofill_view);
    app.settle();

    let field = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Verification code")
        .single();
    assert_eq!(
        field.node().content_type(),
        Some(SnapshotContentType::OneTimeCode),
        "the declared content type must reach the snapshot"
    );
    app.query()
        .content_type(SnapshotContentType::OneTimeCode)
        .assert_exists();
    assert_eq!(
        app.query()
            .role(Role::TEXT_INPUT)
            .label("Note")
            .single()
            .node()
            .content_type(),
        None,
        "a field without a declaration must report no content type"
    );
}

#[test]
fn text_field_content_type_reaches_the_snapshot_on_offscreen_mount() {
    let mut app = ui()
        .theme(Material3::defaults())
        .mount_offscreen(autofill_view);
    app.settle();

    let field = app
        .query()
        .role(Role::TEXT_INPUT)
        .label("Verification code")
        .single();
    assert_eq!(
        field.node().content_type(),
        Some(SnapshotContentType::OneTimeCode),
        "the declared content type must reach the snapshot"
    );
    app.query()
        .content_type(SnapshotContentType::OneTimeCode)
        .assert_exists();
}
