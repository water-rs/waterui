//! Picker Gallery Example - Demonstrates WaterUI's picker components
//!
//! This example showcases:
//! - Picker with different styles (Automatic, Menu, Radio)
//! - DatePicker with typed `jiff` date/time bindings
//! - Calendar for single-date month-grid selection
//! - MultiDatePicker for selecting multiple dates
//! - ColorPicker with alpha and HDR support
//! - FilePicker for file selection and import

use std::{collections::BTreeSet, rc::Rc};

use jiff::civil::{Date, DateTime, Time};
use waterui::Signal;
use waterui::app::App;
use waterui::color::Srgb;
use waterui::component::Dynamic;
use waterui::form::Calendar;
use waterui::form::picker::color::ColorPicker;
use waterui::form::picker::date::{DatePicker, DatePickerType};
use waterui::form::picker::file::FilePicker;
use waterui::form::picker::multi_date::MultiDatePicker;
use waterui::form::picker::{Picker, PickerStyle};
use waterui::media::Url;
use waterui::prelude::*;
use waterui::preview;
use waterui::reactive::binding;
use waterui::shape::{Rectangle, RoundedRectangle, ShapeExt};

const PICKER_BLUE: Srgb = Srgb::from_hex("#3380CC");
const PICKER_PINK: Srgb = Srgb::from_hex("#FF4D80");
const PICKER_RED: Srgb = Srgb::from_hex("#E61A66");

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

fn fixed_date(year: i16, month: i8, day: i8) -> Date {
    Date::new(year, month, day).expect("picker example date must be valid")
}

fn fixed_time(hour: i8, minute: i8, second: i8) -> Time {
    Time::new(hour, minute, second, 0).expect("picker example time must be valid")
}

fn fixed_date_time(year: i16, month: i8, day: i8, hour: i8, minute: i8, second: i8) -> DateTime {
    DateTime::new(year, month, day, hour, minute, second, 0)
        .expect("picker example date-time must be valid")
}

fn decorated_dates() -> BTreeSet<Date> {
    [
        fixed_date(2025, 6, 3),
        fixed_date(2025, 6, 7),
        fixed_date(2025, 6, 15),
        fixed_date(2025, 6, 24),
    ]
    .into_iter()
    .collect()
}

#[preview]
pub fn demo() -> impl View {
    let automatic_selection = binding(Fruit::Apple);
    let menu_selection = binding(Fruit::Banana);
    let radio_selection = binding(Fruit::Cherry);

    let date: Binding<Date> = binding(fixed_date(2025, 1, 1));
    let time_only: Binding<Time> = binding(fixed_time(14, 30, 0));
    let datetime: Binding<DateTime> = binding(fixed_date_time(2025, 6, 15, 9, 45, 30));
    let calendar_date: Binding<Date> = binding(fixed_date(2025, 6, 15));
    let calendar_visible_month: Binding<Date> = binding(fixed_date(2025, 6, 1));
    let available_dates = binding(BTreeSet::<Date>::new());
    let available_visible_month: Binding<Date> = binding(fixed_date(2025, 1, 1));
    let available_date_count = available_dates
        .map(|dates: BTreeSet<Date>| dates.len())
        .computed();
    let decorated_dates = waterui::Computed::constant(decorated_dates());

    let basic_color = binding(Color::from(PICKER_BLUE));
    let alpha_color = binding(Color::from(PICKER_PINK).with_opacity(0.8));
    let hdr_color = binding(Color::from(PICKER_RED));

    let selected_files = binding(Vec::new());

    let picker_items: Vec<_> = Fruit::all()
        .into_iter()
        .map(|(fruit, label)| text(label).tag(fruit))
        .collect();

    scroll(
        vstack((
            vstack((
                text("Picker Gallery").title(),
                text("Demonstrating WaterUI form and picker components").body(),
            )),
            Divider,
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
            vstack((
                text("DatePicker").headline(),
                text("Select dates and times with platform-native pickers").body(),
                spacer(),
                DatePicker::new("Date Only", &date)
                    .range(fixed_date(2025, 1, 1)..=fixed_date(2025, 12, 31)),
                text!("Selected date: {date}"),
                spacer(),
                DatePicker::new("Time Only", &time_only).ty(DatePickerType::HourMinuteAndSecond),
                text!("Selected time: {time_only}"),
                spacer(),
                DatePicker::new("Date & Time", &datetime)
                    .ty(DatePickerType::DateHourMinuteAndSecond),
                text!("Selected datetime: {datetime}"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            vstack((
                text("Calendar").headline(),
                text("Month-grid calendar with single-date selection and passive decorations")
                    .body(),
                spacer(),
                Calendar::new("Trip Date", &calendar_date, &calendar_visible_month)
                    .range(fixed_date(2025, 1, 1)..=fixed_date(2025, 12, 31))
                    .decorated(decorated_dates.clone()),
                text!("Selected calendar date: {calendar_date}"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            vstack((
                text("Multi-Date Picker").headline(),
                text("Month-grid calendar for selecting multiple dates").body(),
                spacer(),
                MultiDatePicker::new(
                    "Available Dates",
                    &available_dates,
                    &available_visible_month,
                )
                .range(fixed_date(2025, 1, 1)..=fixed_date(2025, 12, 31))
                .decorated(decorated_dates),
                text!("Selected dates: {available_date_count}"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            vstack((
                text("ColorPicker").headline(),
                text("Select colors with optional alpha and HDR support").body(),
                spacer(),
                ColorPicker::new("Basic Color", &basic_color),
                color_preview(&basic_color, "Basic"),
                spacer(),
                ColorPicker::new("With Alpha", &alpha_color).with_alpha(),
                color_preview(&alpha_color, "Alpha"),
                spacer(),
                ColorPicker::new("HDR Color", &hdr_color).with_hdr(),
                color_preview(&hdr_color, "HDR"),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            Divider,
            vstack((
                text("FilePicker").headline(),
                text("Select files from the device").body(),
                spacer(),
                FilePicker::open("Select Files", &selected_files).max_count(5),
                spacer(),
                text("Selected files:").bold(),
                file_list(&selected_files),
            ))
            .padding_with(EdgeInsets::all(12.0)),
            vstack((
                Divider,
                text("Built with WaterUI Picker Components").caption(),
            )),
        ))
        .padding_with(EdgeInsets::all(16.0)),
    )
}

fn picker_selection_text(selection: &Binding<Fruit>) -> impl View {
    let selection_text = selection.clone().map(|fruit| format!("{fruit:?}"));
    hstack(("Selected: ", text!("{selection_text}")))
}

fn color_preview(color: &Binding<Color>, label: &'static str) -> impl View {
    hstack((text(label).bold(), text(": "), ColorSwatch::new(color)))
}

struct ColorSwatch {
    color: Binding<Color>,
}

impl ColorSwatch {
    fn new(color: &Binding<Color>) -> Self {
        Self {
            color: color.clone(),
        }
    }
}

impl View for ColorSwatch {
    fn body(self, _env: &Environment) -> impl View {
        signal_driven_view(self.color, |color| {
            Rectangle
                .fill(color)
                .size(64.0, 32.0)
                .clip(RoundedRectangle::new(0.1))
        })
    }
}

fn signal_driven_view<T, S, V>(source: S, build: impl Fn(T) -> V + 'static) -> impl View
where
    S: Signal<Output = T> + Clone + 'static,
    T: 'static,
    V: View,
{
    let (handler, dynamic) = Dynamic::new();
    let build = Rc::new(build);
    handler.set(build(source.get()));

    let guard = source.watch({
        let build = Rc::clone(&build);
        move |ctx| {
            let metadata = ctx.metadata().clone();
            handler.set_with_metadata(build(ctx.into_value()), metadata);
        }
    });

    dynamic.retain((guard, source, build))
}

fn file_list(files: &Binding<Vec<Url>>) -> impl View {
    let file_list_text = files.clone().map(|urls| {
        if urls.is_empty() {
            "No files selected".to_string()
        } else {
            urls.iter()
                .map(|url| format!("- {url}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    });
    text!("{file_list_text}")
}

pub fn app(env: Environment) -> App {
    App::new(demo, env)
}
