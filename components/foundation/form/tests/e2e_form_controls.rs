use jiff::civil::Date;
use std::time::Duration;
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::graphics::color::Srgb;
use waterui::{Binding, View};
use waterui_form::Calendar;
use waterui_form::picker::date::{DatePicker, DatePickerType};
use waterui_form::picker::{Picker, PickerItem, PickerStyle};
use waterui_testing::{MountedApp, Role, UiTest};

const VIEWPORT_WIDTH: u32 = 320;
const VIEWPORT_HEIGHT: u32 = 240;

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
}

#[test]
fn date_picker_accessibility() {
    let selected_date = Binding::container(Date::new(2025, 1, 10).unwrap());
    let selected_date_for_view = selected_date.clone();

    let mut app = mount_view(move || {
        form_shell(vstack((
            DatePicker::new(&selected_date_for_view).label(waterui::text!("Event Date")),
            waterui::text!("selected:{selected_date_for_view}").foreground(Srgb::WHITE),
        )))
    });

    let initial_value = DatePickerType::Date.format_value(selected_date.get().at(0, 0, 0, 0));
    app.query()
        .role(Role::COMBOBOX)
        .value(initial_value.clone())
        .assert_exists();

    let updated_date = Date::new(2025, 2, 14).unwrap();
    let updated_value = DatePickerType::Date.format_value(updated_date.at(0, 0, 0, 0));
    assert!(
        app.query()
            .role(Role::COMBOBOX)
            .value(initial_value)
            .set_text(updated_value.clone()),
        "date picker set_text should succeed"
    );
    assert_eq!(
        selected_date.get(),
        updated_date,
        "date picker should update binding"
    );
    app.query()
        .role(Role::COMBOBOX)
        .value(updated_value)
        .assert_exists();
    app.query()
        .role(Role::LABEL)
        .label("selected:2025-02-14")
        .assert_exists();
}

#[test]
fn calendar_navigation_and_selection_update_binding() {
    let selected_date = Binding::container(Date::new(2025, 1, 10).unwrap());
    let selected_date_for_view = selected_date.clone();

    let mut app = mount_view(move || {
        form_shell(
            Calendar::new(&selected_date_for_view)
                .label("Event Calendar")
                .range(Date::new(2025, 1, 1).unwrap()..=Date::new(2025, 2, 28).unwrap()),
        )
    });

    app.query()
        .role(Role::LABEL)
        .label("Event Calendar")
        .assert_exists();
    app.query().role(Role::BUTTON).label("14").assert_exists();
    assert!(
        app.query().role(Role::BUTTON).label("14").tap(),
        "calendar should allow selecting a visible in-range day"
    );
    assert_eq!(
        selected_date.get(),
        Date::new(2025, 1, 14).unwrap(),
        "calendar day tap should update the selected date"
    );

    assert!(
        app.query().role(Role::BUTTON).label(">").tap(),
        "calendar next-month button should be tappable"
    );
    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("31")
            .wait_for_nonexistence(Duration::from_secs(1)),
        "calendar should rebuild to the next month before a new day selection"
    );
    assert!(
        app.query().role(Role::BUTTON).label("20").tap(),
        "calendar should allow selecting a day after month navigation"
    );
    assert_eq!(
        selected_date.get(),
        Date::new(2025, 2, 20).unwrap(),
        "calendar month navigation should change the active month before selection"
    );
}
