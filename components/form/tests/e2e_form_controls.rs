use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::graphics::color::Srgb;
use waterui::{Binding, View};
use waterui_form::picker::{Picker, PickerItem, PickerStyle};
use waterui_testing::{MountedApp, Role, UiTest};

const VIEWPORT_WIDTH: u32 = 320;
const VIEWPORT_HEIGHT: u32 = 240;
const SUITE: &str = "form/controls";

fn mount_view<V, F>(build: F) -> MountedApp
where
    V: View + 'static,
    F: Fn() -> V + 'static,
{
    UiTest::new()
        .viewport(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
        .mount(build)
}

fn form_shell<V: View>(content: V) -> impl View {
    vstack((content,))
        .spacing(12.0)
        .padding_with(16.0)
        .background(Srgb::BLACK)
}

fn picker_items() -> Vec<PickerItem<&'static str>> {
    vec![
        waterui::text!("Alpha").tag("Alpha"),
        waterui::text!("Beta").tag("Beta"),
        waterui::text!("Gamma").tag("Gamma"),
    ]
}

#[test]
fn picker_selection_flow() {
    let selection = Binding::container("Alpha");
    let selection_for_view = selection.clone();

    let mut app = mount_view(move || {
        form_shell(vstack((
            Picker::new(picker_items(), &selection_for_view).style(PickerStyle::Menu),
            waterui::text!("selected:{selection_for_view}").foreground(Srgb::WHITE),
        )))
    });

    let initial = app.capture_snapshot(SUITE, "picker-selection-flow", "00_initial");
    assert!(
        initial.path().is_file(),
        "picker-selection-flow: initial snapshot missing"
    );

    app.query()
        .role(Role::COMBOBOX)
        .value("Alpha")
        .assert_exists();
    app.query()
        .role(Role::OPTION)
        .label("Alpha")
        .assert_exists();
    app.query().role(Role::OPTION).label("Beta").assert_exists();
    app.query()
        .role(Role::OPTION)
        .label("Gamma")
        .assert_exists();

    assert!(
        app.query().role(Role::OPTION).label("Beta").tap(),
        "picker option selection should succeed"
    );
    assert_eq!(
        selection.get(),
        "Beta",
        "picker selection should update binding"
    );
    app.query()
        .role(Role::COMBOBOX)
        .value("Beta")
        .assert_exists();
    app.query()
        .role(Role::OPTION)
        .label("Beta")
        .selected(true)
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("selected:Beta")
        .assert_exists();

    let selected = app.capture_snapshot(SUITE, "picker-selection-flow", "01_selected");
    assert!(
        selected.path().is_file(),
        "picker-selection-flow: selected snapshot missing"
    );
    assert!(
        initial.snapshot().changed_pixels(selected.snapshot()) > 0,
        "picker selection should change rendered pixels"
    );
}
