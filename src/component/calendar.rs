use crate::View;

use crate::hstack;
use crate::reactive::IntoRef;
use crate::reactive::Ref;
use crate::view::BoxView;
use crate::view::Frame;
use crate::view::ViewExt;
use chrono::Local;
use chrono::NaiveDate;

use super::button;
use super::foreach::ForEach;

pub struct Calendar {
    frame: Frame,
    date: Ref<NaiveDate>,
}

impl Calendar {
    pub fn new(date: impl IntoRef<NaiveDate>) -> Self {
        Self {
            frame: Frame::default(),
            date: date.into_ref(),
        }
    }

    pub fn now() -> Self {
        Self::new(Local::now().date_naive())
    }
}

impl View for Calendar {
    fn view(&self) -> BoxView {
        ForEach::new(1..4, |_n| {
            ForEach::new(1..=7, |n| hstack![button(n.to_string())]).horizontal()
        })
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

    use crate::html::HtmlRenderer;

    use super::Calendar;

    #[test]
    fn test() {
        let s = HtmlRenderer::new().renderer(Box::new(Calendar::now()));
        println!("{s}");
    }
}
