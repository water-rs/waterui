//! End-to-end accessibility-semantics tests for form controls.

use jiff::civil::Date;
use std::time::Duration;
use waterui::ViewExt as _;
use waterui::component::vstack;
use waterui::graphics::color::Srgb;
use waterui::{Binding, Color, View};
use waterui_form::Calendar;
use waterui_form::picker::color::ColorPicker;
use waterui_form::picker::date::{DatePicker, DatePickerType};
use waterui_form::picker::{Picker, PickerItem, PickerStyle};
use waterui_testing::{Role, UiBuilder};

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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn picker_selection_flow(ui: UiBuilder) {
    let selection = Binding::container("Alpha");
    let selection_for_view = selection.clone();

    let mut app = ui.mount(move || {
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

    app.query().role(Role::OPTION).label("Beta").tap();
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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn picker_initial_non_first_selection_uses_matching_item_id(ui: UiBuilder) {
    let selection = Binding::container("Beta");
    let selection_for_view = selection.clone();

    let mut app = ui.mount(move || {
        form_shell(Picker::new(picker_items(), &selection_for_view).style(PickerStyle::Menu))
    });

    app.query()
        .role(Role::COMBOBOX)
        .value("Beta")
        .assert_exists();
    app.query()
        .role(Role::OPTION)
        .label("Beta")
        .selected(true)
        .assert_exists();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn date_picker_accessibility(ui: UiBuilder) {
    let selected_date = Binding::container(Date::new(2025, 1, 10).unwrap());
    let selected_date_for_view = selected_date.clone();

    let mut app = ui.mount(move || {
        form_shell(vstack((
            DatePicker::new(waterui::text!("Event Date"), &selected_date_for_view),
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
    app.query()
        .role(Role::COMBOBOX)
        .value(initial_value)
        .set_text(updated_value.clone());
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

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn color_picker_accessibility_tap_is_handled(ui: UiBuilder) {
    let selected_color = Binding::container(Color::srgb(0, 0, 0));
    let selected_color_for_view = selected_color;

    let mut app = ui.mount(move || {
        form_shell(ColorPicker::new(
            waterui::text!("Accent Color"),
            &selected_color_for_view,
        ))
    });

    app.query().role(Role::BUTTON).label("Accent Color").tap();
}

#[waterui::test(theme = hydrolysis_m3::install, viewport = (320, 240))]
fn calendar_navigation_and_selection_update_binding(ui: UiBuilder) {
    let selected_date = Binding::container(Date::new(2025, 1, 10).unwrap());
    let visible_month = Binding::container(Date::new(2025, 1, 1).unwrap());
    let selected_date_for_view = selected_date.clone();
    let visible_month_for_view = visible_month;

    let mut app = ui.mount(move || {
        form_shell(
            Calendar::new(
                "Event Calendar",
                &selected_date_for_view,
                &visible_month_for_view,
            )
            .range(Date::new(2025, 1, 1).unwrap()..=Date::new(2025, 2, 28).unwrap()),
        )
    });

    app.query()
        .role(Role::LABEL)
        .label("Event Calendar")
        .assert_exists();
    app.query().role(Role::BUTTON).label("14").assert_exists();
    app.query().role(Role::BUTTON).label("14").tap();
    assert_eq!(
        selected_date.get(),
        Date::new(2025, 1, 14).unwrap(),
        "calendar day tap should update the selected date"
    );

    app.query().role(Role::BUTTON).label(">").tap();
    assert!(
        app.query()
            .role(Role::BUTTON)
            .label("31")
            .wait_for_nonexistence(Duration::from_secs(1)),
        "calendar should rebuild to the next month before a new day selection"
    );
    app.query().role(Role::BUTTON).label("20").tap();
    assert_eq!(
        selected_date.get(),
        Date::new(2025, 2, 20).unwrap(),
        "calendar month navigation should change the active month before selection"
    );
}
