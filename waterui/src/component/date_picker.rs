use super::{text, vstack, Button, HStack, Text, VStack};
use crate::Environment;
use crate::{view::ViewExt, Binding, View};
use chrono::{Datelike, Days, NaiveDate, Weekday};
use itertools::Itertools;

pub struct DatePicker {
    date: Binding<NaiveDate>,
}

impl DatePicker {
    pub fn new(date: &Binding<NaiveDate>) -> Self {
        Self { date: date.clone() }
    }
}

impl View for DatePicker {
    fn body(self, _env: Environment) -> impl View {
        let first_day = self.date.get().with_day(1).unwrap();
        let weekday = Days::new(first_day.weekday().num_days_from_monday() as u64);
        let day = first_day - weekday;
        let day_iter = day.iter_days().take(5 * 7);
        let day_iter = day_iter
            .into_iter()
            .map(|date| (date, date.day0()))
            .chunks(7);
        let days = day_iter
            .into_iter()
            .map(|chunk| chunk.collect_vec())
            .collect_vec();
        let mut day_iter = vec![vec![(NaiveDate::MIN, 0); days.len()]; days[0].len()];
        for (i, row) in days.iter().enumerate() {
            for (j, col) in row.iter().enumerate() {
                day_iter[j][i] = *col;
            }
        }

        let day_iter = day_iter.into_iter().enumerate().map(|(n, chunk)| {
            vstack((
                Text::display(Weekday::try_from(n as u8).unwrap())
                    .size(13)
                    .disable_select(),
                VStack::from_iter(chunk.into_iter().map(|(button_date, v)| {
                    let date = self.date.clone();
                    Button::new(Text::display(v).width(30).height(30), move || {
                        date.set(button_date)
                    })
                })),
            ))
        });

        vstack((
            text(self.date.get().format("%B %e").to_string())
                .bold()
                .leading(),
            HStack::from_iter(day_iter),
        ))
    }
}
