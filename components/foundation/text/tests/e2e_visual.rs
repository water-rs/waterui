use waterui::ViewExt as _;
use waterui::accessibility::AccessibilityRole;
use waterui::graphics::color::Srgb;
use waterui::text::{styled, text};
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
