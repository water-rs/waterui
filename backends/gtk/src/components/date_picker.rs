//! GTK4 DatePicker component implementation.

use gtk4::prelude::*;
use gtk4::{Calendar, Widget};
use nami::Signal;
use waterui_core::{Environment, Native};
use waterui_form::picker::date::{Date, DatePickerConfig, Month};

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
            let month = Month::try_from(
                u8::try_from(selected.month()).expect("GTK Calendar month must fit u8"),
            )
            .expect("GTK Calendar month must be a valid time::Month");
            let day = u8::try_from(selected.day_of_month())
                .expect("GTK Calendar day must fit u8 and be positive");
            let date = Date::from_calendar_date(selected.year(), month, day)
                .expect("GTK Calendar selected date must be representable");
            value.set(date);
        });

        let guard = config.value.watch({
            let calendar = calendar.clone();
            move |ctx: nami::watcher::Context<Date>| {
                let date = ctx.into_value();
                let calendar = calendar.clone();
                glib::idle_add_local_once(move || {
                    apply_calendar_date(&calendar, date);
                });
            }
        });

        root.append(&calendar);
        store_watcher_guard(&root, Box::new(guard));
        root.upcast()
    }
}

fn apply_calendar_date(calendar: &Calendar, date: Date) {
    let (year, month, day) = date.to_calendar_date();
    let month = i32::from(month as u8);
    let day = i32::from(day);
    let date_time = glib::DateTime::from_local(year, month, day, 0, 0, 0.0)
        .expect("DatePicker date must be valid for glib::DateTime");
    calendar.select_day(&date_time);
}
