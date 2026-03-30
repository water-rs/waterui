//! Integration smoke tests proving `waterui-testing` runs under ordinary `cargo test`.

use waterui::ViewExt as _;
use waterui::component::text;
use waterui::graphics::color::Srgb;
use waterui_testing::{Role, UiTest};

fn sample_view() -> impl waterui::View {
    text("Macro-mounted label")
        .body()
        .foreground(Srgb::WHITE)
        .background(Srgb::BLACK)
}

#[waterui::test(sample_view)]
fn waterui_test_macro_runs_inside_cargo_test(app: &mut waterui_testing::MountedApp) {
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single();
    assert_eq!(label.node().label(), Some("Macro-mounted label"));
}

#[test]
fn direct_ui_test_mount_runs_inside_cargo_test() {
    let mut app = UiTest::new().viewport(240, 120).mount(sample_view);
    let label = app
        .query()
        .role(Role::LABEL)
        .label("Macro-mounted label")
        .single();
    assert_eq!(label.node().label(), Some("Macro-mounted label"));
}
