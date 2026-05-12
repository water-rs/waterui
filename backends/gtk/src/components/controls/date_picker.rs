//! GTK4 DatePicker component implementation.

use gtk4::prelude::*;
use gtk4::{Calendar, Widget};
use nami::Signal;
use waterui_core::{Environment, Native};
use waterui_form::picker::date::{Date, DatePickerConfig, DateTime};

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guard;

impl GtkComponent for Native<DatePickerConfig> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let label = renderer.render_any(config.label, env);
        root.append(&label);

        let calendar = Calendar::new();
        apply_calendar_date(&calendar, config.value.get());

        let value = config.value.clone();
        calendar.connect_day_selected(move |calendar| {
            let selected = calendar.date();
            let date = Date::new(
                i16::try_from(selected.year()).expect("GTK Calendar year must fit jiff::Date"),
                i8::try_from(selected.month()).expect("GTK Calendar month must fit jiff::Date"),
                i8::try_from(selected.day_of_month())
                    .expect("GTK Calendar day must fit jiff::Date"),
            )
            .expect("GTK Calendar selected date must be representable");
            let current = value.get();
            let time = current.time();
            value.set(date.at(
                time.hour(),
                time.minute(),
                time.second(),
                time.subsec_nanosecond(),
            ));
        });

        let guard = config.value.watch({
            let calendar = calendar.clone();
            move |ctx: nami::watcher::Context<DateTime>| {
                let date_time = ctx.into_value();
                let calendar = calendar.clone();
                glib::idle_add_local_once(move || {
                    apply_calendar_date(&calendar, date_time);
                });
            }
        });

        root.append(&calendar);
        store_watcher_guard(&root, Box::new(guard));
        root.upcast()
    }
}

fn apply_calendar_date(calendar: &Calendar, date_time: DateTime) {
    let date = date_time.date();
    let year = i32::from(date.year());
    let month = i32::from(date.month());
    let day = i32::from(date.day());
    let date_time = glib::DateTime::from_local(year, month, day, 0, 0, 0.0)
        .expect("DatePicker date must be valid for glib::DateTime");
    calendar.select_day(&date_time);
}
