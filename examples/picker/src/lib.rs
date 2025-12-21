//! Picker Gallery Example - Demonstrates WaterUI's picker components
//!
//! This example showcases:
//! - DatePicker with various date/time selection modes
//! - ColorPicker with alpha and HDR support
//! - FilePicker for file selection and import

use time::{Date, Month};
use waterui::app::App;
use waterui::form::picker::color::ColorPicker;
use waterui::form::picker::date::{DatePicker, DatePickerType};
use waterui::form::picker::file::FilePicker;
use waterui::media::Url;
use waterui::prelude::*;
use waterui::reactive::binding;
use waterui::shape::{Rectangle, RoundedRectangle, ShapeExt};

fn main() -> impl View {
    // DatePicker bindings
    let date = binding(Date::from_calendar_date(2025, Month::January, 1).unwrap());
    let datetime = binding(Date::from_calendar_date(2025, Month::June, 15).unwrap());

    // ColorPicker bindings
    let basic_color = binding(Color::srgb_f32(0.2, 0.5, 0.8));
    let alpha_color = binding(Color::srgb_f32(1.0, 0.3, 0.5).with_alpha(0.8));
    let hdr_color = binding(Color::srgb_f32(0.9, 0.1, 0.4));

    // FilePicker binding
    let selected_files = binding(Vec::new());

    scroll(
        vstack((
            // Header
            text("Picker Gallery").size(28.0),
            "Demonstrating WaterUI's picker components",
            Divider,
            spacer(),
            // Section 1: DatePicker
            vstack((
                text("DatePicker").size(20.0),
                "Select dates and times with platform-native pickers",
                spacer(),
                // Date only picker
                DatePicker::new(&date)
                    .label(text("Date Only"))
                    .ty(DatePickerType::Date),
                hstack(("Selected date: ", text!("{date}"))),
                spacer(),
                // Date and time picker
                DatePicker::new(&datetime)
                    .label(text("Date & Time"))
                    .ty(DatePickerType::DateHourAndMinute),
                hstack(("Selected datetime: ", text!("{datetime}"))),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            spacer(),
            // Section 2: ColorPicker
            vstack((
                text("ColorPicker").size(20.0),
                "Select colors with optional alpha and HDR support",
                spacer(),
                // Basic color picker
                ColorPicker::new(&basic_color).label(text("Basic Color")),
                color_preview(&basic_color, "Basic"),
                spacer(),
                // Color picker with alpha
                ColorPicker::new(&alpha_color)
                    .label(text("With Alpha"))
                    .support_alpha(true),
                color_preview(&alpha_color, "Alpha"),
                spacer(),
                // HDR color picker (iOS 14+)
                ColorPicker::new(&hdr_color)
                    .label(text("HDR Color"))
                    .support_hdr(true),
                color_preview(&hdr_color, "HDR"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            spacer(),
            // Section 3: FilePicker
            vstack((
                text("FilePicker").size(20.0),
                "Select files from the device",
                spacer(),
                // Single file picker
                FilePicker::open(&selected_files).num(5),
                spacer(),
                // Show selected files
                text("Selected files:").bold(),
                file_list(&selected_files),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            spacer(),
            Divider,
            "Built with WaterUI Picker Components",
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
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
