use crate::utils::Color;
use crate::Binding;
use crate::View;

use super::button;
use super::foreach::ForEach;
use super::text;
use super::Button;
use crate::hstack;
use crate::view::BoxView;
use crate::view::Frame;
use crate::view::ViewExt;
use crate::vstack;
use chrono::DateTime;
use chrono::Local;
use chrono::NaiveDate;
use chrono::Weekday;
use text::Text;

pub struct Calendar {
    frame: Frame,
    date: Binding<DateTime<Local>>,
}

impl Calendar {
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

impl View for Calendar {
    fn view(&mut self) -> BoxView {
        vstack![
            text(self.date.get().format("%B %e").to_string()).bold(),
            ForEach::new(1..=7, |weekday| {
                ForEach::new(1..=4, |week| {
                    Button::display(week * 7 + weekday).height(30).width(30)
                })
            })
            .horizontal()
        ]
        .into_boxed()
    }

    fn frame(&self) -> crate::view::Frame {
        self.frame.clone()
    }
    fn set_frame(&mut self, frame: crate::view::Frame) {
        self.frame = frame
    }
}

mod test {

    use std::time::Instant;

    use crate::html::HtmlRenderer;

    use super::Calendar;

    #[test]
    fn test() {
        let start = Instant::now();

        let s = HtmlRenderer::new().renderer(Box::new(Calendar::now()));
        let duration = start.elapsed();
        println!("{s}");
        println!("Duration:{duration:?}");
    }
}
