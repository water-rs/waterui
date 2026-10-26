//! `#[widget]` end-to-end: the fixture crate's declarations land as
//! `waterui_widget_*` exports paired with `waterui_meta_widget_*` records,
//! and `apple::widgets::declared_widgets` pairs them back into the
//! declarations the extension build consumes.

use std::path::{Path, PathBuf};
use std::process::Command;

use waterui_cli::apple::widgets::declared_widgets;
use waterui_cli::artifact_symbols::ArtifactSymbols;

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/widget_app")
}

fn build_fixture() -> PathBuf {
    let fixture = fixture_dir();
    let status = Command::new(env!("CARGO"))
        .current_dir(&fixture)
        .args(["build", "--offline"])
        .env("CARGO_TARGET_DIR", fixture.join("target"))
        .status()
        .expect("build the widget fixture");
    assert!(status.success(), "the widget fixture must build");
    fixture.join("target/debug/libwidget_app.a")
}

#[test]
fn widget_declarations_pair_exports_with_records() {
    let artifact = build_fixture();
    let symbols = ArtifactSymbols::read(&artifact).expect("read the fixture artifact");

    let widgets = declared_widgets(&symbols).expect("pair declarations with exports");
    assert_eq!(widgets.len(), 2);

    // Sorted by `kind`: `now-kind` before `steps`.
    let now = &widgets[0];
    assert_eq!(now.function, "now");
    assert_eq!(now.kind, "now-kind");
    assert_eq!(now.display_name, "Now");
    assert_eq!(now.description, "The current time");
    assert_eq!(now.families, ["systemSmall"]);
    assert_eq!(now.symbol, "waterui_widget_widget_app_now");

    let steps = &widgets[1];
    assert_eq!(steps.function, "steps");
    assert_eq!(steps.kind, "steps");
    assert_eq!(steps.display_name, "Steps");
    assert_eq!(steps.description, "Today's step count");
    assert_eq!(
        steps.families,
        ["systemSmall", "systemMedium", "systemLarge"]
    );
    assert_eq!(steps.symbol, "waterui_widget_widget_app_steps");

    // The `waterui-widgets` static record the extension build locates the
    // shipped Swift sources through rides the same artifact.
    let sources = symbols
        .meta_record("waterui_meta_widgets_swift_sources")
        .expect("the swift sources record is in the artifact");
    let sources = String::from_utf8(sources).expect("the record is UTF-8");
    assert!(
        Path::new(&sources).join("FFI.swift").is_file(),
        "the record points at the crate's Swift sources: {sources}"
    );
}
