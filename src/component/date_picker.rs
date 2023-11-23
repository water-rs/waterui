use crate::Binding;
use crate::View;

use super::foreach::ForEach;
use super::text;
use super::Button;
use super::Stack;
use crate::view::BoxView;
use crate::view::Frame;
use crate::view::ViewExt;
use crate::vstack;
use chrono::DateTime;
use chrono::Datelike;
use chrono::Days;
use chrono::Local;
use chrono::NaiveDate;
use chrono::Weekday;
use itertools::Itertools;
use text::Text;

pub struct DatePicker {
    frame: Frame,
    date: Binding<DateTime<Local>>,
}

impl DatePicker {
    pub fn new(date: impl Into<Binding<DateTime<Local>>>) -> Self {
        Self {
            frame: Frame::default(),
            date: date.into(),
        }
    }

    pub fn now() -> Self {
        Self::new(Local::now())
    }
}

impl View for DatePicker {
    fn view(&mut self) -> BoxView {
        let first_day = self.date.get().with_day(1).unwrap();
        let weekday = Days::new(first_day.weekday().num_days_from_monday() as u64);
        let day = first_day - weekday;
        let day_iter: Vec<NaiveDate> = day.date_naive().iter_days().take(5 * 7).collect_vec();
        let day_iter = day_iter.into_iter().map(|date| date.day0()).chunks(7);
        let day_iter = day_iter.into_iter().map(|chunk| {
            Stack::new(chunk.map(|v| Button::display(v).width(30).height(30))).horizontal()
        });

        vstack![
            text(self.date.get().format("%B %e").to_string()).bold(),
            ForEach::new(0..7, |n| Text::display(Weekday::try_from(n).unwrap())).horizontal(),
            Stack::new(day_iter)
        ]
        .into_boxed()
    }

    fn frame(&self) -> crate::view::Frame {
        self.frame.clone()
    }
    fn set_frame(&mut self, frame: crate::view::Frame) {
        self.frame = frame
    }

    fn is_reactive(&self) -> bool {
        true
    }

    fn subscribe(&self, subscriber: fn() -> crate::binding::BoxSubscriber) {
        self.date.add_boxed_subscriber((subscriber)());
    }
}

#[cfg(test)]
mod test {

    use std::time::Instant;

    use crate::html::HtmlRenderer;

    use super::DatePicker;

    #[test]
    fn test() {
        let start = Instant::now();

        let s = HtmlRenderer::new().renderer(Box::new(DatePicker::now()));
        let duration = start.elapsed();
        println!("{s}");
        println!("Duration:{duration:?}");
    }
}
