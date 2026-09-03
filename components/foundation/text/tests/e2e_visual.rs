//! End-to-end visual-rendering tests for the `text` component.

use hydrolysis_m3::{MaterialColorScheme, install, install_with_colors};
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui::text::{code, styled, text};
use waterui::{Environment, ViewExt as _};
use waterui_testing::{OffscreenApp, Role, SemanticApp};

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

/// The Material 3 baseline dark scheme, so a code block has to read every
/// colour from the environment to stay legible.
fn dark_theme(env: &mut Environment) {
    install_with_colors(env, MaterialColorScheme::baseline_dark());
}

fn assert_code_block_semantics(app: &mut OffscreenApp) {
    app.query().role(Role::LABEL).label("Rust").assert_exists();
    app.query().role(Role::LABEL).label("Copy").assert_exists();
}

#[waterui::test(code_block, theme = install, viewport = (480, 300), offscreen)]
fn a_code_block_draws_from_the_light_theme(app: &mut OffscreenApp) {
    assert_code_block_semantics(app);
    let _ = app.capture_snapshot("text", "code_block", "light");
}

#[waterui::test(code_block, theme = dark_theme, viewport = (480, 300), offscreen)]
fn a_code_block_draws_from_the_dark_theme(app: &mut OffscreenApp) {
    assert_code_block_semantics(app);
    let _ = app.capture_snapshot("text", "code_block", "dark");
}
