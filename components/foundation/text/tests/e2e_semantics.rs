//! End-to-end semantic tests for the `text` component.

use std::cell::Cell;
use std::rc::Rc;

use waterui::Binding;
use waterui::Str;
use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui::text::highlight::Language;
use waterui::text::{code, styled, text};
use waterui_testing::{Role, SemanticApp, UiBuilder};

fn plain_text_view() -> impl waterui::View {
    text("Visible content")
        .body()
        .foreground(Srgb::WHITE)
        .padding_with(16.0)
        .background(Srgb::BLACK)
        .a11y_role(AccessibilityRole::Text)
}

fn styled_text_view() -> impl waterui::View {
    text(styled::StyledStr::from_markdown(
        "Plain *italic* **bold** `code`",
    ))
    .body()
    .foreground(Srgb::WHITE)
    .padding_with(16.0)
    .background(Srgb::BLACK)
    .a11y_role(AccessibilityRole::Text)
}

#[waterui::test(plain_text_view)]
fn text_renders_visible_content(app: &mut SemanticApp) {
    app.query()
        .role(Role::LABEL)
        .label("Visible content")
        .assert_exists();
}

#[waterui::test(styled_text_view)]
fn styled_text_renders_multiple_styles(app: &mut SemanticApp) {
    app.query()
        .role(Role::LABEL)
        .label("Plain italic bold code")
        .assert_exists();
}

fn code_block() -> impl waterui::View {
    code("rust", include_str!("fixtures/code_sample.rs")).padding_with(16.0)
}

/// The semantic half of the code-block captures: a `Code` widget names its
/// language and publishes its copy affordance, neither of which needs a
/// rendered frame to be true.
#[waterui::test(code_block)]
fn a_code_block_publishes_its_language_and_copy_action(app: &mut SemanticApp) {
    app.query().role(Role::LABEL).label("Rust").assert_exists();
    app.query().role(Role::BUTTON).label("Copy").assert_exists();
}

/// The copy affordance is a real button: it shows up in the tree under the
/// BUTTON role named for its label, and activating it runs the copy — which
/// the block reports through `on_copied`.
#[waterui::test()]
fn activating_the_copy_button_copies(ui: UiBuilder) {
    let copied = Rc::new(Cell::new(false));
    let observed = Rc::clone(&copied);
    let mut app = ui.mount(move || {
        let observed = Rc::clone(&observed);
        code(Language::Rust, "fn main() {}")
            .on_copied(move |_| observed.set(true))
            .padding_with(16.0)
    });

    app.query().role(Role::BUTTON).label("Copy").assert_exists();
    app.query().role(Role::BUTTON).label("Copy").tap();
    assert!(copied.get(), "activating Copy must run on_copied");
}

#[waterui::test()]
fn unknown_language_code_block_shows_original_token_in_header(ui: UiBuilder) {
    let mut app = ui.mount(|| code("mermaid", "graph TD; A-->B").padding_with(16.0));
    app.query()
        .role(Role::LABEL)
        .label("mermaid")
        .assert_exists();
    app.query().role(Role::BUTTON).label("Copy").assert_exists();
}

#[waterui::test()]
fn mutating_bound_code_content_rehighlights_in_place(ui: UiBuilder) {
    let source = Binding::new(Str::from("fn alpha() {}"));
    let mut app = ui.mount({
        let source = source.clone();
        move || code(Language::Rust, source.clone()).padding_with(16.0)
    });

    app.query().role(Role::LABEL).label("Rust").assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("fn alpha() {}")
        .assert_exists();

    source.set(Str::from("fn beta() {}"));
    app.query()
        .role(Role::LABEL)
        .label("fn beta() {}")
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("fn alpha() {}")
        .assert_not_exists();
    app.query().role(Role::LABEL).label("Rust").assert_exists();
}
