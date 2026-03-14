//! GTK4 multi-date picker component implementation.

use gtk4::prelude::*;
use gtk4::{Calendar, Widget};
use nami::Signal;
use waterui_core::{Environment, Native};
use waterui_form::picker::date::Date;
use waterui_form::picker::multi_date::MultiDatePickerConfig;

use crate::component::GtkComponent;
use crate::renderer::GtkRenderer;
use crate::util::store_watcher_guards;

impl GtkComponent for Native<MultiDatePickerConfig> {
    fn render(self, env: &Environment, renderer: &mut GtkRenderer) -> Widget {
        let config = self.into_inner();

        let root = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
        let label = renderer.render_any(config.label, env);
        root.append(&label);

        let calendar = Calendar::new();
        let value = config.value.clone();
        let decorated = config.decorated.clone();

        apply_multi_date_state(&calendar, &value.get(), &decorated.get());

        calendar.connect_day_selected({
            let calendar = calendar.clone();
            let value = value.clone();
            move |_| {
                let selected = calendar.date();
                let date = Date::new(
                    selected.year() as i16,
                    selected.month() as i8 + 1,
                    selected.day_of_month() as i8,
                )
                .expect("GTK Calendar selected date must be representable");
                let mut dates = value.get();
                if let Some(index) = dates.iter().position(|existing| *existing == date) {
                    dates.remove(index);
                } else {
                    dates.push(date);
                    dates.sort();
                    dates.dedup();
                }
                value.set(dates);
            }
        });

        calendar.connect_notify_local(Some("month"), {
            let calendar = calendar.clone();
            let value = value.clone();
            let decorated = decorated.clone();
            move |_, _| {
                apply_multi_date_state(&calendar, &value.get(), &decorated.get())
            }
        });
        calendar.connect_notify_local(Some("year"), {
            let calendar = calendar.clone();
            let value = value.clone();
            let decorated = decorated.clone();
            move |_, _| {
                apply_multi_date_state(&calendar, &value.get(), &decorated.get())
            }
        });

        let value_guard = value.watch({
            let calendar = calendar.clone();
            let decorated = decorated.clone();
            move |ctx: nami::watcher::Context<Vec<Date>>| {
                let dates = ctx.into_value();
                let decorated_dates = decorated.get();
                let calendar = calendar.clone();
                glib::idle_add_local_once(move || {
                    apply_multi_date_state(&calendar, &dates, &decorated_dates);
                });
            }
        });

        let decorated_guard = decorated.watch({
            let calendar = calendar.clone();
            let value = value.clone();
            move |ctx: nami::watcher::Context<Vec<Date>>| {
                let decorated_dates = ctx.into_value();
                let selected_dates = value.get();
                let calendar = calendar.clone();
                glib::idle_add_local_once(move || {
                    apply_multi_date_state(&calendar, &selected_dates, &decorated_dates);
                });
            }
        });

        root.append(&calendar);
        store_watcher_guards(&root, vec![Box::new(value_guard), Box::new(decorated_guard)]);
        root.upcast()
    }
}

fn apply_multi_date_state(calendar: &Calendar, selected: &[Date], decorated: &[Date]) {
    for day in 1..=31 {
        calendar.unmark_day(day);
    }

    let visible = calendar.date();
    let visible_year = visible.year() as i16;
    let visible_month = visible.month() as i8 + 1;

    for date in selected.iter().chain(decorated.iter()) {
        if date.year() == visible_year && date.month() == visible_month {
            calendar.mark_day(date.day().into());
        }
    }

    if let Some(first_selected) = selected
        .iter()
        .find(|date| date.year() == visible_year && date.month() == visible_month)
    {
        let date_time = glib::DateTime::from_local(
            i32::from(first_selected.year()),
            i32::from(first_selected.month()),
            i32::from(first_selected.day()),
            0,
            0,
            0.0,
        )
        .expect("multi-date picker selection must be valid for glib::DateTime");
        calendar.select_day(&date_time);
    }
}
