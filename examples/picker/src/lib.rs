//! Picker Gallery Example - Demonstrates WaterUI's picker components
//!
//! This example showcases:
//! - Picker with different styles (Automatic, Menu, Radio)
//! - DatePicker with various date/time selection modes
//! - ColorPicker with alpha and HDR support
//! - FilePicker for file selection and import

use time::{Date, Month};
use waterui::app::App;
use waterui::form::picker::color::ColorPicker;
use waterui::form::picker::date::{DatePicker, DatePickerType};
use waterui::form::picker::file::FilePicker;
use waterui::form::picker::{Picker, PickerStyle};
use waterui::media::Url;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{Rectangle, RoundedRectangle, ShapeExt};

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
    let datetime = binding(Date::from_calendar_date(2025, Month::June, 15).unwrap());

    // ColorPicker bindings
    let basic_color = binding(Color::srgb_f32(0.2, 0.5, 0.8));
    let alpha_color = binding(Color::srgb_f32(1.0, 0.3, 0.5).with_alpha(0.8));
    let hdr_color = binding(Color::srgb_f32(0.9, 0.1, 0.4));

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
                text("Picker Gallery").size(28.0),
                "Demonstrating WaterUI's picker components",
            )),
            Divider,
            // Section 0: Picker Styles
            vstack((
                text("Picker Styles").size(20.0),
                "Choose from different picker presentation styles",
                spacer(),
                text("Automatic (default)").bold(),
                Picker::new(picker_items.clone(), &automatic_selection),
                picker_selection_text(&automatic_selection),
                spacer(),
                text("Menu Style").bold(),
                Picker::new(picker_items.clone(), &menu_selection)
                    .style(PickerStyle::Menu),
                picker_selection_text(&menu_selection),
                spacer(),
                text("Radio Style").bold(),
                Picker::new(picker_items.clone(), &radio_selection)
                    .style(PickerStyle::Radio),
                picker_selection_text(&radio_selection),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 1: DatePicker
            vstack((
                text("DatePicker").size(20.0),
                "Select dates and times with platform-native pickers",
                spacer(),
                DatePicker::new(&date)
                    .label(text("Date Only"))
                    .ty(DatePickerType::Date),
                hstack(("Selected date: ", text!("{date}"))),
                spacer(),
                DatePicker::new(&datetime)
                    .label(text("Date & Time"))
                    .ty(DatePickerType::DateHourAndMinute),
                hstack(("Selected datetime: ", text!("{datetime}"))),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 2: ColorPicker
            vstack((
                text("ColorPicker").size(20.0),
                "Select colors with optional alpha and HDR support",
                spacer(),
                ColorPicker::new(&basic_color).label(text("Basic Color")),
                color_preview(&basic_color, "Basic"),
                spacer(),
                ColorPicker::new(&alpha_color)
                    .label(text("With Alpha"))
                    .support_alpha(true),
                color_preview(&alpha_color, "Alpha"),
                spacer(),
                ColorPicker::new(&hdr_color)
                    .label(text("HDR Color"))
                    .support_hdr(true),
                color_preview(&hdr_color, "HDR"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            // Section 3: FilePicker
            vstack((
                text("FilePicker").size(20.0),
                "Select files from the device",
                spacer(),
                FilePicker::open(&selected_files).num(5),
                spacer(),
                text("Selected files:").bold(),
                file_list(&selected_files),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            // Footer
            vstack((Divider, "Built with WaterUI Picker Components")),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

/// Helper view to display picker selection
fn picker_selection_text(selection: &Binding<Fruit>) -> impl View {
    let selection = selection.clone();
    hstack((
        "Selected: ",
        text(selection.map(|fruit| format!("{fruit:?}"))),
    ))
}

/// Helper view to display a color preview
fn color_preview(color: &Binding<Color>, label: &'static str) -> impl View {
    let color = color.clone();
    hstack((
        text(label).bold(),
        ": ",
        watch(color, |c: Color| {
            Rectangle
                .fill(c)
                .size(64.0, 32.0)
                .clip(RoundedRectangle::new(0.1))
        }),
    ))
}

/// Helper view to display selected files
fn file_list(files: &Binding<Vec<Url>>) -> impl View {
    let files = files.clone();
    text(files.map(|urls| {
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
