use super::foreach::ForEach;
use super::stack::vstack;
use super::text;
use super::Button;
use super::Stack;
use crate::view::Frame;
use crate::view::ViewExt;
use crate::widget;
use crate::Binding;
use crate::View;
use chrono::DateTime;
use chrono::Datelike;
use chrono::Days;
use chrono::Local;
use chrono::NaiveDate;
use chrono::Weekday;
use itertools::Itertools;
use text::Text;

#[widget]
pub struct DatePicker {
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

#[widget]
impl View for DatePicker {
    fn view(&mut self) -> Stack {
        let first_day = self.date.get().with_day(1).unwrap();
        let weekday = Days::new(first_day.weekday().num_days_from_monday() as u64);
        let day = first_day - weekday;
        let day_iter: Vec<NaiveDate> = day.date_naive().iter_days().take(5 * 7).collect_vec();
        let day_iter = day_iter.into_iter().map(|date| date.day0()).chunks(5);
        let day_iter = day_iter.into_iter().enumerate().map(|(n, chunk)| {
            vstack((
                Text::display(Weekday::try_from(n as u8).unwrap()).size(13),
                Stack::from_iter(chunk.map(|v| Button::display(v).width(30).height(30))),
            ))
        });

        vstack((
            text(self.date.get().format("%B %e").to_string())
                .bold()
                .leading(),
            Stack::from_iter(day_iter).horizontal(),
        ))
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
