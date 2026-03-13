//! Picker Gallery Example - Demonstrates WaterUI's picker components
//!
//! This example showcases:
//! - Picker with different styles (Automatic, Menu, Radio)
//! - DatePicker with various date/time selection modes
//! - ColorPicker with alpha and HDR support
//! - MultiDatePicker for selecting multiple dates
//! - FilePicker for file selection and import

use std::collections::BTreeSet;
use time::{Date, Month, PrimitiveDateTime, Time};
use waterui::app::App;
use waterui::color::Srgb;
use waterui::form::picker::color::ColorPicker;
use waterui::form::picker::date::{DatePicker, DatePickerType};
use waterui::form::picker::file::FilePicker;
use waterui::form::picker::multi_date::MultiDatePicker;
use waterui::form::picker::{Picker, PickerStyle};
use waterui::media::Url;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::RoundedRectangle;

// Color constants for picker defaults
const PICKER_BLUE: Srgb = Srgb::from_hex("#3380CC");
const PICKER_PINK: Srgb = Srgb::from_hex("#FF4D80");
const PICKER_RED: Srgb = Srgb::from_hex("#E61A66");

/// Fruit options for picker demos
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
enum Fruit {
    #[default]
    Apple,
    Banana,
    Cherry,
    Date,
    Elderberry,
}

impl Fruit {
    fn all() -> Vec<(Self, &'static str)> {
        vec![
            (Self::Apple, "Apple"),
            (Self::Banana, "Banana"),
            (Self::Cherry, "Cherry"),
            (Self::Date, "Date"),
            (Self::Elderberry, "Elderberry"),
        ]
    }
}

fn main() -> impl View {
    // Picker bindings for style demos
    let automatic_selection = binding(Fruit::Apple);
    let menu_selection = binding(Fruit::Banana);
    let radio_selection = binding(Fruit::Cherry);

    // DatePicker bindings
    let date = binding(Date::from_calendar_date(2025, Month::January, 1).unwrap());
    let time_only = binding(Time::from_hms(14, 30, 0).unwrap());
    let datetime = binding(PrimitiveDateTime::new(
        Date::from_calendar_date(2025, Month::June, 15).unwrap(),
        Time::from_hms(9, 45, 30).unwrap(),
    ));
    let available_dates = binding(BTreeSet::<Date>::new());
    let available_date_count = available_dates.map(|dates| dates.len()).computed();

    // ColorPicker bindings
    let basic_color = binding(Color::from(PICKER_BLUE));
    let alpha_color = binding(Color::from(PICKER_PINK).with_opacity(0.8));
    let hdr_color = binding(Color::from(PICKER_RED));

    // FilePicker binding
    let selected_files = binding(Vec::new());

    // Create picker items
    let picker_items: Vec<_> = Fruit::all()
        .into_iter()
        .map(|(fruit, label)| text(label).tag(fruit))
        .collect();

    scroll(
        vstack((
            // Header section
            vstack((
                text("Picker Gallery").title(),
                text("Demonstrating WaterUI's picker components").body(),
            )),
            Divider,
            // Section 0: Picker Styles
            vstack((
                text("Picker Styles").headline(),
                text("Choose from different picker presentation styles").body(),
                spacer(),
                text("Automatic (default)").bold(),
                Picker::new(picker_items.clone(), &automatic_selection),
                picker_selection_text(&automatic_selection),
                spacer(),
                text("Menu Style").bold(),
                Picker::new(picker_items.clone(), &menu_selection).style(PickerStyle::Menu),
                picker_selection_text(&menu_selection),
                spacer(),
                text("Radio Style").bold(),
                Picker::new(picker_items.clone(), &radio_selection).style(PickerStyle::Radio),
                picker_selection_text(&radio_selection),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 1: DatePicker
            vstack((
                text("DatePicker").headline(),
                text("Select dates and times with platform-native pickers").body(),
                spacer(),
                DatePicker::new(&date).label("Date Only").range(
                    Date::from_calendar_date(2025, Month::January, 1).unwrap()
                        ..=Date::from_calendar_date(2025, Month::December, 31).unwrap(),
                ),
                text!("Selected date: {date}"),
                spacer(),
                DatePicker::time(&time_only)
                    .label("Time Only")
                    .ty(DatePickerType::HourMinuteAndSecond),
                text!("Selected time: {time_only}"),
                spacer(),
                DatePicker::datetime(&datetime)
                    .label("Date & Time")
                    .ty(DatePickerType::DateHourMinuteAndSecond),
                text!("Selected datetime: {datetime}"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 2: ColorPicker
            vstack((
                text("ColorPicker").headline(),
                text("Select colors with optional alpha and HDR support").body(),
                spacer(),
                ColorPicker::new(&basic_color).label("Basic Color"),
                color_preview(&basic_color, "Basic"),
                spacer(),
                ColorPicker::new(&alpha_color)
                    .label("With Alpha")
                    .support_alpha(true),
                color_preview(&alpha_color, "Alpha"),
                spacer(),
                ColorPicker::new(&hdr_color)
                    .label("HDR Color")
                    .support_hdr(true),
                color_preview(&hdr_color, "HDR"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 3: MultiDatePicker
            vstack((
                text("MultiDatePicker").headline(),
                text("Select multiple dates in a cross-platform calendar").body(),
                spacer(),
                MultiDatePicker::new(&available_dates)
                    .label("Availability")
                    .range(
                        Date::from_calendar_date(2025, Month::January, 1).unwrap()
                            ..=Date::from_calendar_date(2025, Month::December, 31).unwrap(),
                    ),
                text!("Selected dates: {available_date_count}"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 4: FilePicker
            vstack((
                text("FilePicker").headline(),
                text("Select files from the device").body(),
                spacer(),
                FilePicker::open(&selected_files).num(5),
                spacer(),
                text("Selected files:").bold(),
                file_list(&selected_files),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            // Footer
            vstack((
                Divider,
                text("Built with WaterUI Picker Components").caption(),
            )),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// Helper view to display picker selection
fn picker_selection_text(selection: &Binding<Fruit>) -> impl View {
    let selection = selection.clone();
    hstack((
        "Selected: ",
        Text::computed(selection.map(|fruit| format!("{fruit:?}"))),
    ))
}

/// Helper view to display a color preview
fn color_preview(color: &Binding<Color>, label: &'static str) -> impl View {
    use waterui::shape::{Rectangle, ShapeExt};
    hstack((
        text(label).bold(),
        text(": "),
        color
            .clone()
            .map(|c| {
                Rectangle
                    .fill(c)
                    .size(64.0, 32.0)
                    .clip(RoundedRectangle::new(0.1))
            })
            .computed(),
    ))
}

/// Helper view to display selected files
fn file_list(files: &Binding<Vec<Url>>) -> impl View {
    let files = files.clone();
    Text::computed(files.map(|urls| {
        if urls.is_empty() {
            "No files selected".to_string()
        } else {
            urls.iter()
                .map(|url| format!("- {url}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }))
}

pub fn app(env: Environment) -> App {
    App::new(main(), env)
}

waterui_ffi::export!();
