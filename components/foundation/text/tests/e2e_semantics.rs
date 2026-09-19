//! End-to-end semantic tests for the `text` component.

use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui::text::{code, styled, text};
use waterui_testing::{Role, SemanticApp};

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
    app.query().role(Role::LABEL).label("Copy").assert_exists();
}